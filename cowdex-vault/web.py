#!/usr/bin/env python3
"""Password-unlocked local browser manager for the Cowdex credential broker."""

import argparse
import copy
import ctypes
import fcntl
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import http.client
import json
import os
from pathlib import Path
import resource
import secrets
import socket
import socketserver
import subprocess
import sys
import threading
import time
import webbrowser

import broker
from approvals import Pending, policy
from templates import TEMPLATES


class Manager:
    def __init__(self, home, ttl, terminate=os._exit):
        self.home, self.ttl, self.terminate = home, ttl, terminate
        self.vault = home / "vault.enc"
        self.profile = home / "grant.json"
        self.socket = home / "manager.sock"
        self.key = None
        self.credentials = {}
        self.expires_at = None
        self.admin_token = None
        self.ticket = None
        self.sessions = {}
        self.pending = {}
        self.next_attempt = 0
        self.guard = threading.RLock()

    def stop(self):
        self.profile.unlink(missing_ok=True)
        self.socket.unlink(missing_ok=True)
        self.terminate(0)

    def atomic_write(self, path, raw):
        if len(raw) > broker.MAX_VAULT:
            raise broker.Refused()
        temporary = path.with_name(path.name + "." + secrets.token_hex(8))
        try:
            broker.write_private(temporary, raw)
            os.replace(temporary, broker.private_path(path))
        finally:
            temporary.unlink(missing_ok=True)

    def publish(self):
        aliases = sorted(self.credentials)
        self.ticket = secrets.token_urlsafe(32) if aliases else None
        if aliases:
            self.atomic_write(self.profile, json.dumps({"socket": str(self.socket),
                              "ticket": self.ticket, "scope": aliases[:8]}).encode())
        else:
            self.profile.unlink(missing_ok=True)

    def authorized(self, token):
        return (self.admin_token is not None and isinstance(token, str)
                and secrets.compare_digest(token.encode(), self.admin_token.encode())
                and time.time() < self.expires_at)

    def state(self, token):
        self.purge()
        if not self.authorized(token):
            return {"unlocked": False, "exists": self.vault.exists(), "templates": TEMPLATES,
                    "approvalsRequired": True}
        return {"unlocked": True, "expiresAt": self.expires_at,
                "approvalsRequired": True, **self.approvals(),
                "templates": TEMPLATES,
                "credentials": [{"alias": alias, "origin": c["origin"],
                                 "prefix": c["prefix"], "methods": c["methods"],
                                 "template": c.get("template", "custom"),
                                 "auth": c.get("auth", "bearer"),
                                 "body_format": c.get("body_format", "json"),
                                 "hasUsername": bool(c.get("username"))}
                                for alias, c in sorted(self.credentials.items())],
                "ready": self.ticket is not None}

    def purge(self):
        now = time.time()
        for claim, pending in list(self.pending.items()):
            if now >= pending.expires_at or (pending.approved_until is not None
                                            and now >= pending.approved_until):
                del self.pending[claim]
        for session, active in list(self.sessions.items()):
            if now >= active.expires_at:
                active.revoked = True
                active.credentials.clear()
                del self.sessions[session]

    def approvals(self):
        self.purge()
        return {"approvals": [pending.public() for pending in self.pending.values()]}

    def decide(self, code, approve):
        self.purge()
        if not isinstance(code, str):
            raise broker.Refused()
        for claim, pending in self.pending.items():
            if pending.code == code:
                if approve:
                    if pending.approved_until is None:
                        pending.approved_until = min(int(time.time()) + 86400, self.expires_at)
                else:
                    del self.pending[claim]
                return self.approvals()
        raise broker.Refused()

    def unlock(self, body):
        if time.time() < self.next_attempt:
            raise broker.Refused()
        self.next_attempt = time.time() + 1
        password = body.get("password")
        if not isinstance(password, str) or not 1 <= len(password) <= 1024:
            raise broker.Refused()
        if not self.vault.exists():
            if password != body.get("confirmPassword"):
                raise broker.Refused()
            broker.write_private(self.vault, broker.encrypt({}, password))
        raw = broker.read_private(self.vault)
        key = broker.file_key(raw, password)
        credentials = broker.decrypt_with_key(raw, key)
        del password
        if self.expires_at is None:
            self.expires_at = int(time.time()) + self.ttl
            deadline = threading.Timer(self.ttl, self.stop)
            deadline.daemon = True
            deadline.start()
        if time.time() >= self.expires_at:
            raise broker.Refused()
        self.key, self.salt, self.credentials = key, raw[4:20], credentials
        self.admin_token = secrets.token_urlsafe(32)
        self.publish()
        return {**self.state(self.admin_token), "adminToken": self.admin_token}

    def save(self, body):
        alias = body.get("alias")
        if not isinstance(alias, str) or not broker.ALIAS.fullmatch(alias):
            raise broker.Refused()
        template_id = body.get("template", "custom")
        template = next((t for t in TEMPLATES if t["id"] == template_id), None)
        if template is None:
            raise broker.Refused()
        credential = {"template": template_id, "shared": True,
                      "prefix": body.get("prefix", "/"),
                      "methods": body.get("methods", sorted(broker.METHODS))}
        for field, default in (("origin", None), ("auth", "bearer"), ("body_format", "json")):
            credential[field] = body.get(field, default) if template_id == "custom" else template[field]
        previous = self.credentials.get(alias)
        same_destination = previous is not None and all(
            previous.get(field, default) == credential[field]
            for field, default in (("origin", None), ("auth", "bearer"),
                                   ("body_format", "json"), ("template", "custom")))
        for field in ("token", "username") if credential["auth"] in ("basic", "helio") else ("token",):
            value = body.get(field, "")
            credential[field] = previous.get(field, "") if value == "" and same_destination else value
        if "fixed_username" in template:
            credential["username"] = template["fixed_username"]
        broker.validate_credential(alias, credential)
        updated = {**self.credentials, alias: credential}
        if len(updated) > 8 and alias not in self.credentials:
            raise broker.Refused()
        nonce = os.urandom(12)
        payload = broker.AESGCM(self.key).encrypt(nonce, json.dumps(updated).encode(), b"cowdex-vault-v1")
        self.atomic_write(self.vault, b"CV01" + self.salt + nonce + payload)
        if previous is not None and policy(previous) != policy(credential):
            for claim, pending in list(self.pending.items()):
                if alias in pending.policies:
                    del self.pending[claim]
            for session, active in list(self.sessions.items()):
                if alias in active.credentials:
                    active.revoked = True
                    active.credentials.clear()
                    del self.sessions[session]
        self.credentials = updated
        self.publish()
        return self.state(self.admin_token)

    def dispatch(self, request):
        with self.guard:
            if self.key is None or time.time() >= self.expires_at or not isinstance(request, dict):
                raise broker.Refused()
            self.purge()
            if request.get("op") == "attach":
                ticket, scope = request.get("ticket"), request.get("scope")
                if (not isinstance(ticket, str) or self.ticket is None
                        or not secrets.compare_digest(ticket, self.ticket)
                        or not isinstance(scope, list) or not 1 <= len(scope) <= 8
                        or any(not isinstance(a, str) for a in scope)
                        or len(set(scope)) != len(scope)
                        or any(a not in self.credentials for a in scope)
                        or len(self.sessions) >= 64 or len(self.pending) >= 8):
                    raise broker.Refused()
                code = secrets.token_hex(6).upper()
                while any(pending.code == code for pending in self.pending.values()):
                    code = secrets.token_hex(6).upper()
                pending = Pending(secrets.token_urlsafe(32), code,
                                  {a: policy(self.credentials[a]) for a in scope},
                                  min(int(time.time()) + 300, self.expires_at))
                self.publish()
                self.pending[pending.claim_token] = pending
                return pending.response()
            if request.get("op") in ("claim", "cancel"):
                claim = request.get("claim_token")
                if not isinstance(claim, str):
                    raise broker.Refused()
                if request["op"] == "cancel":
                    self.pending.pop(claim, None)
                    return {}
                pending = self.pending.get(claim)
                if pending is None:
                    raise broker.Refused()
                if any(a not in self.credentials or policy(self.credentials[a]) != frozen
                       for a, frozen in pending.policies.items()):
                    del self.pending[claim]
                    raise broker.Refused()
                if pending.approved_until is None:
                    return pending.response()
                if len(self.sessions) >= 64:
                    raise broker.Refused()
                current = broker.decrypt_with_key(broker.read_private(self.vault), self.key)
                if any(a not in current or policy(current[a]) != frozen
                       for a, frozen in pending.policies.items()):
                    del self.pending[claim]
                    raise broker.Refused()
                selected = copy.deepcopy({a: current[a] for a in pending.policies})
                active = broker.Broker(selected, claim, pending.approved_until,
                                       lambda: broker.decrypt_with_key(broker.read_private(self.vault), self.key))
                result = active.handle({"op": "attach", "ticket": claim, "scope": sorted(selected)})
                del self.pending[claim]
                self.sessions[result["session"]] = active
                return result
            session = request.get("session")
            if not isinstance(session, str) or session not in self.sessions:
                raise broker.Refused()
            result = self.sessions[session].handle(request)
            if request.get("op") == "detach":
                del self.sessions[session]
            return result


class HttpHandler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, status, result):
        raw = result if isinstance(result, bytes) else json.dumps(result).encode()
        self.send_response(status)
        self.send_header("Content-Type", "text/html; charset=utf-8" if isinstance(result, bytes) else "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("X-Frame-Options", "DENY")
        self.send_header("Content-Security-Policy", "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'")
        self.send_header("X-Cowdex-Vault", "1")
        self.end_headers()
        self.wfile.write(raw)

    def session_token(self):
        authorization = self.headers.get("Authorization", "")
        return authorization.removeprefix("Bearer ") if authorization.startswith("Bearer ") else None

    def do_GET(self):
        if self.headers.get("Host") != self.server.authority:
            self.reply(403, {"error": "Vault operation refused."})
        elif self.path == "/":
            self.reply(200, Path(__file__).with_name("index.html").read_bytes())
        elif self.path == "/api/state":
            with self.server.manager.guard:
                self.reply(200, self.server.manager.state(self.session_token()))
        elif self.path == "/api/approvals":
            with self.server.manager.guard:
                manager = self.server.manager
                if manager.authorized(self.session_token()):
                    self.reply(200, manager.approvals())
                else:
                    self.reply(403, {"error": "Vault operation refused."})
        elif self.path.startswith("/api/proof?challenge="):
            challenge = self.path.removeprefix("/api/proof?challenge=")
            if len(challenge) != 64 or any(c not in "0123456789abcdef" for c in challenge):
                self.reply(403, {"error": "Vault operation refused."})
                return
            self.reply(200, {"proof": hmac.new(self.server.proof, challenge.encode(), hashlib.sha256).hexdigest()})
        else:
            self.reply(404, {"error": "Not found."})

    def do_POST(self):
        try:
            if (self.headers.get("Host") != self.server.authority
                    or self.headers.get("Origin") != "http://" + self.server.authority
                    or self.headers.get("Content-Type") != "application/json"
                    or self.headers.get("Transfer-Encoding") is not None):
                raise broker.Refused()
            size = int(self.headers.get("Content-Length", "0"))
            if not 1 <= size <= broker.MAX_MESSAGE:
                raise broker.Refused()
            self.connection.settimeout(5)
            body = json.loads(self.rfile.read(size))
            if not isinstance(body, dict):
                raise broker.Refused()
            manager = self.server.manager
            with manager.guard:
                if self.path == "/api/unlock":
                    result = manager.unlock(body)
                    self.reply(200, result)
                else:
                    if not manager.authorized(self.session_token()):
                        raise broker.Refused()
                    if self.path == "/api/save":
                        self.reply(200, manager.save(body))
                    elif self.path in ("/api/approve", "/api/deny"):
                        self.reply(200, manager.decide(body.get("code"), self.path == "/api/approve"))
                    elif self.path == "/api/lock":
                        self.reply(200, {"unlocked": False})
                        self.wfile.flush()
                        manager.stop()
                    else:
                        raise broker.Refused()
        except Exception:
            self.reply(403, {"error": "Vault operation refused. Check your password and fields."})


class SocketHandler(socketserver.StreamRequestHandler):
    def handle(self):
        self.connection.settimeout(5)
        try:
            line = self.rfile.readline(broker.MAX_MESSAGE + 1)
            if len(line) > broker.MAX_MESSAGE or not line.endswith(b"\n"):
                raise broker.Refused()
            result = self.server.manager.dispatch(json.loads(line))
        except Exception:
            result = {"error": "vault operation refused"}
        self.wfile.write(json.dumps(result).encode() + b"\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serve", action="store_true")
    parser.add_argument("--port", type=int, default=2749)
    parser.add_argument("--ttl", type=int, default=86400)
    parser.add_argument("--inherited-http-fd", type=int, help=argparse.SUPPRESS)
    parser.add_argument("--inherited-lock-fd", type=int, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if sys.platform != "linux" or not 1 <= args.ttl <= 86400:
        parser.error("Linux and an unlock duration of 1–86400 seconds are required")
    os.umask(0o077)
    home = Path(os.environ.get("COWDEX_VAULT_HOME", "~/.local/share/cowdex-vault")).expanduser().absolute()
    home.mkdir(mode=0o700, parents=True, exist_ok=True)
    broker.private_path(home / "vault.enc")
    authority = f"127.0.0.1:{args.port}"
    if not args.serve:
        child = None
        for attempt in range(50):
            try:
                proof = broker.read_private(home / "manager.proof")
                challenge = secrets.token_hex(32)
                connection = http.client.HTTPConnection("127.0.0.1", args.port, timeout=1)
                connection.request("GET", "/api/proof?challenge=" + challenge)
                response = connection.getresponse()
                expected = hmac.new(proof, challenge.encode(), hashlib.sha256).hexdigest()
                if response.status != 200 or json.loads(response.read(4096)).get("proof") != expected:
                    raise broker.Refused()
                connection.close()
                webbrowser.open("http://" + authority)
                print("Vault manager: http://" + authority)
                return
            except OSError:
                if attempt == 0:
                    child = subprocess.Popen([sys.executable, __file__, "--serve", "--port", str(args.port),
                                              "--ttl", str(args.ttl)], start_new_session=True,
                                             stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                             stderr=subprocess.DEVNULL)
                if child is not None and child.poll() is not None:
                    raise broker.Refused()
                time.sleep(0.1)
        raise broker.Refused()
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    if ctypes.CDLL(None).prctl(4, 0, 0, 0, 0) != 0:
        raise broker.Refused()
    lock = args.inherited_lock_fd
    if lock is None:
        lock = os.open(broker.private_path(home / "manager.lock"), os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def restart(_code):
        descriptor = http_server.fileno()
        os.set_inheritable(descriptor, True)
        os.set_inheritable(lock, True)
        try:
            os.execv(sys.executable, [sys.executable, __file__, "--serve", "--port", str(args.port),
                                     "--ttl", str(args.ttl), "--inherited-http-fd", str(descriptor),
                                     "--inherited-lock-fd", str(lock)])
        except BaseException:
            os._exit(1)

    manager = Manager(home, args.ttl, restart)
    if args.inherited_http_fd is None:
        manager.atomic_write(home / "manager.proof", secrets.token_bytes(32))
    with ThreadingHTTPServer(("127.0.0.1", args.port), HttpHandler, bind_and_activate=False) as http_server:
        if args.inherited_http_fd is not None:
            http_server.socket.close()
            http_server.socket = socket.socket(fileno=args.inherited_http_fd)
        manager.socket.unlink(missing_ok=True)
        manager.profile.unlink(missing_ok=True)
        with socketserver.ThreadingUnixStreamServer(str(manager.socket), SocketHandler) as unix:
            os.chmod(manager.socket, 0o600)
            unix.daemon_threads = True
            unix.manager = manager
            http_server.manager, http_server.authority = manager, authority
            http_server.proof = broker.read_private(home / "manager.proof")
            if args.inherited_http_fd is None:
                http_server.server_bind()
                http_server.server_activate()
            threading.Thread(target=unix.serve_forever, daemon=True).start()
            http_server.serve_forever()


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("Could not open vault manager. Check private paths and whether the selected port is available.", file=sys.stderr)
        sys.exit(1)
