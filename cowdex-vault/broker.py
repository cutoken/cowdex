#!/usr/bin/env python3
"""Small human-unlocked credential broker; Linux Unix sockets only."""

import argparse
import base64
import ctypes
import errno
import getpass
import grp
import http.client
import json
import math
import os
from pathlib import Path
import pwd
import re
import resource
import secrets
import socket
import ssl
import stat
import sys
import threading
import time
from urllib.parse import parse_qsl, urlencode, urlsplit

from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.scrypt import Scrypt

MAX_MESSAGE = 65536
MAX_RESPONSE = 8192
MAX_VAULT = 1048576
ALIAS = re.compile(r"[A-Za-z0-9_-]{1,32}\Z")
METHODS = {"GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"}


class Refused(Exception):
    """A failed operation whose details must not cross the broker boundary."""


def private_path(value):
    path = Path(value)
    if not path.is_absolute() or ".." in path.parts:
        raise Refused()
    for parent in reversed(path.parents):
        info = parent.lstat()
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            raise Refused()
        # Root-owned ancestors such as /tmp are acceptable only when sticky.
        if info.st_mode & 0o022 and not (info.st_uid == 0 and info.st_mode & stat.S_ISVTX):
            # Common ~/.local directories are group-writable by a user-private
            # group. Accept that only when it grants no other account access.
            if info.st_mode & 0o002 or info.st_uid != os.getuid():
                raise Refused()
            owner = pwd.getpwuid(info.st_uid)
            group = grp.getgrgid(info.st_gid)
            if (group.gr_name != owner.pw_name
                    or any(member != owner.pw_name for member in group.gr_mem)
                    or any(user.pw_gid == info.st_gid and user.pw_uid != info.st_uid
                           for user in pwd.getpwall())):
                raise Refused()
            try:
                os.getxattr(parent, "system.posix_acl_access", follow_symlinks=False)
            except OSError as error:
                if error.errno not in {errno.ENODATA, errno.ENOTSUP}:
                    raise Refused() from error
            else:
                # Named ACL entries can grant writes outside the owning group.
                raise Refused()
    if path.parent.stat().st_uid != os.getuid():
        raise Refused()
    return path


def read_private(path):
    fd = os.open(private_path(path), os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, "rb") as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise Refused()
        raw = stream.read(MAX_VAULT + 1)
        if len(raw) > MAX_VAULT:
            raise Refused()
        return raw


def write_private(path, data):
    fd = os.open(private_path(path), os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def file_key(raw, password):
    if raw[:4] != b"CV01" or len(raw) < 48:
        raise Refused()
    return Scrypt(salt=raw[4:20], length=32, n=2**15, r=8, p=1).derive(password.encode())


def encrypt(credentials, password, salt=None):
    salt, nonce = salt or os.urandom(16), os.urandom(12)
    key = Scrypt(salt=salt, length=32, n=2**15, r=8, p=1).derive(password.encode())
    payload = AESGCM(key).encrypt(nonce, json.dumps(credentials).encode(), b"cowdex-vault-v1")
    return b"CV01" + salt + nonce + payload


def decrypt(raw, password):
    return decrypt_with_key(raw, file_key(raw, password))


def decrypt_with_key(raw, key):
    if raw[:4] != b"CV01" or len(raw) < 48:
        raise Refused()
    result = json.loads(AESGCM(key).decrypt(raw[20:32], raw[32:], b"cowdex-vault-v1"))
    if not isinstance(result, dict):
        raise Refused()
    for alias, credential in result.items():
        validate_credential(alias, credential)
    return result


def canonical_path(path):
    # Deliberately exclude encoded/ambiguous routing and fragments. Query values
    # can be percent encoded, but cannot change the path used by authorization.
    if not isinstance(path, str) or not path.startswith("/") or path.startswith("//"):
        raise Refused()
    if any(ord(c) < 33 or ord(c) > 126 for c in path) or "\\" in path or "#" in path:
        raise Refused()
    route = path.split("?", 1)[0]
    if "%" in route or "//" in route or any(part in {".", ".."} for part in route.split("/")):
        raise Refused()
    return route


def validate_credential(alias, credential):
    if not isinstance(alias, str) or not ALIAS.fullmatch(alias) or not isinstance(credential, dict):
        raise Refused()
    origin = urlsplit(credential["origin"])
    if origin.scheme != "https" or not origin.hostname or origin.username or origin.password:
        raise Refused()
    if origin.path or origin.query or origin.fragment or any(c.isspace() for c in credential["origin"]):
        raise Refused()
    if origin.port not in {None, 443}:
        raise Refused()
    token = credential["token"]
    if not isinstance(token, str) or not 8 <= len(token) <= 4096 or any(ord(c) < 33 or ord(c) > 126 for c in token):
        raise Refused()
    if credential.get("auth", "bearer") not in ("bearer", "basic", "helio"):
        raise Refused()
    if credential.get("body_format", "json") not in ("json", "form", "multipart"):
        raise Refused()
    if credential.get("auth") in ("basic", "helio"):
        username = credential.get("username")
        if (not isinstance(username, str) or not 1 <= len(username) <= 4096
                or ":" in username or any(ord(c) < 33 or ord(c) > 126 for c in username)):
            raise Refused()
    methods = credential["methods"]
    if not isinstance(methods, list) or not methods or any(m not in METHODS for m in methods):
        raise Refused()
    prefix = credential["prefix"]
    if canonical_path(prefix) != prefix:
        raise Refused()


def upstream(credential, method, path, body):
    validate_credential("request", credential)
    origin = urlsplit(credential["origin"])
    conn = http.client.HTTPSConnection(origin.hostname, timeout=20, context=ssl.create_default_context())
    try:
        secret = credential["token"]
        authorization = "Bearer " + secret
        redactions = [secret.encode()]
        if credential.get("auth") == "basic":
            encoded = base64.b64encode((credential["username"] + ":" + secret).encode())
            authorization = "Basic " + encoded.decode("ascii")
            redactions.insert(0, encoded)
        headers = {"Authorization": authorization, "Accept-Encoding": "identity", "User-Agent": "Cowdex-Vault"}
        if credential.get("auth") == "helio":
            route, _, query = path.partition("?")
            fields = [(key, value) for key, value in parse_qsl(query, keep_blank_values=True) if key != "apiKey"]
            fields.append(("apiKey", credential["username"]))
            path = route + "?" + urlencode(fields)
            redactions.append(credential["username"].encode())
        if body is not None:
            headers["Content-Type"] = "application/json"
            if credential.get("body_format") in ("form", "multipart"):
                try:
                    fields = json.loads(body)
                except (ValueError, TypeError) as error:
                    raise Refused() from error
                if not isinstance(fields, dict):
                    raise Refused()
                pairs = []
                for key, value in fields.items():
                    for item in value if isinstance(value, list) else [value]:
                        if (not isinstance(item, (str, int, float, bool))
                                or isinstance(item, float) and not math.isfinite(item)):
                            raise Refused()
                        pairs.append((key, str(item).lower() if isinstance(item, bool) else item))
                body = urlencode(pairs)
                headers["Content-Type"] = "application/x-www-form-urlencoded"
                if credential["body_format"] == "multipart":
                    if any(any(char in key for char in '\r\n"\\') for key, _ in pairs):
                        raise Refused()
                    boundary = "cowdex-" + secrets.token_hex(24)
                    body = "".join(f'--{boundary}\r\nContent-Disposition: form-data; name="{key}"\r\n\r\n{value}\r\n'
                                   for key, value in pairs) + f"--{boundary}--\r\n"
                    headers["Content-Type"] = "multipart/form-data; boundary=" + boundary
        conn.request(method, path, body=body.encode() if body is not None else None, headers=headers)
        response = conn.getresponse()
        if 300 <= response.status < 400 or response.getheader("Content-Encoding", "identity") != "identity":
            raise Refused()
        raw = response.read(MAX_RESPONSE + 1)
        # Reject oversized responses in full: never return a partial secret.
        if len(raw) > MAX_RESPONSE:
            raise Refused()
        for sensitive in redactions:
            raw = raw.replace(sensitive, b"[REDACTED]")
        if len(raw) > MAX_RESPONSE:
            raise Refused()
        return {"status": response.status, "body": raw.decode("utf-8", errors="replace"), "truncated": False}
    finally:
        conn.close()


class Broker:
    """One immutable credential scope shared by one attached session family."""

    def __init__(self, credentials, ticket, expires_at, reload_credentials=None):
        self.credentials = credentials
        self.ticket = ticket
        self.expires_at = expires_at
        self.session = None
        self.revoked = False
        self.reload_credentials = reload_credentials

    def handle(self, request):
        if time.time() >= self.expires_at or self.revoked or not isinstance(request, dict):
            raise Refused()
        operation = request.get("op")
        public = {"aliases": sorted(self.credentials), "expires_at": self.expires_at}
        if operation == "attach":
            scope = request.get("scope")
            ticket = request.get("ticket")
            if not isinstance(ticket, str) or self.ticket is None or not secrets.compare_digest(ticket, self.ticket):
                raise Refused()
            if not isinstance(scope, list) or any(not isinstance(a, str) for a in scope):
                raise Refused()
            if len(scope) != len(self.credentials) or set(scope) != set(self.credentials):
                raise Refused()
            self.ticket = None
            self.session = secrets.token_urlsafe(32)
            return {"session": self.session, **public}
        supplied = request.get("session")
        if not isinstance(supplied, str) or self.session is None or not secrets.compare_digest(supplied, self.session):
            raise Refused()
        if operation == "status":
            return public
        if operation == "detach":
            self.revoked = True
            self.session = None
            self.credentials.clear()
            return {}
        if operation != "request" or set(request) - {"op", "session", "credential", "method", "path", "body"}:
            raise Refused()
        credential = self.credentials.get(request.get("credential"))
        if credential is None or request.get("method") not in credential["methods"]:
            raise Refused()
        route = canonical_path(request.get("path"))
        prefix = credential["prefix"].rstrip("/")
        if route != prefix and not route.startswith(prefix + "/"):
            raise Refused()
        body = request.get("body")
        if body is not None and not isinstance(body, str):
            raise Refused()
        if self.reload_credentials is not None:
            current = self.reload_credentials()
            for alias, frozen in self.credentials.items():
                updated = current.get(alias)
                if updated is None or any(updated.get(field, default) != frozen.get(field, default)
                        for field, default in (("origin", None), ("auth", "bearer"),
                                               ("username", None), ("body_format", "json"))):
                    raise Refused()
            for alias, frozen in self.credentials.items():
                frozen["token"] = current[alias]["token"]
        result = upstream(credential, request["method"], request["path"], body)
        if time.time() >= self.expires_at:
            raise Refused()
        return result


def serve(args, key, credentials):
    socket_path, grant_path = private_path(args.socket), private_path(args.grant_file)
    if socket_path.exists() or socket_path.is_symlink() or grant_path.exists() or grant_path.is_symlink():
        raise Refused()
    scope = args.scope.split(",")
    if not 1 <= len(scope) <= 8 or len(set(scope)) != len(scope) or any(not ALIAS.fullmatch(a) for a in scope):
        raise Refused()
    selected = {alias: credentials[alias] for alias in scope}
    credentials.clear()
    ticket = secrets.token_urlsafe(32)

    def reload_credentials():
        current = decrypt_with_key(read_private(args.vault), key)
        return {alias: current[alias] for alias in scope}

    broker = Broker(selected, ticket, int(time.time()) + args.ttl, reload_credentials)
    # A hard process lifetime bounds Python-managed key copies as well as live
    # requests. This is process destruction, not a Python zeroization promise.
    def expire():
        try:
            socket_path.unlink(missing_ok=True)
            grant_path.unlink(missing_ok=True)
        finally:
            os._exit(0)

    deadline = threading.Timer(args.ttl, expire)
    deadline.daemon = True
    deadline.start()
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
        listener.bind(str(socket_path))
        try:
            if args.socket_group:
                os.chown(socket_path, -1, grp.getgrnam(args.socket_group).gr_gid)
            os.chmod(socket_path, 0o660)
            listener.listen(8)
            write_private(grant_path, json.dumps({"socket": str(socket_path), "ticket": ticket}).encode())
            while not broker.revoked:
                connection, _ = listener.accept()
                with connection:
                    connection.settimeout(5)
                    try:
                        with connection.makefile("rb") as stream:
                            line = stream.readline(MAX_MESSAGE + 1)
                        if len(line) > MAX_MESSAGE or not line.endswith(b"\n"):
                            raise Refused()
                        result = broker.handle(json.loads(line))
                    except Exception:
                        result = {"error": "vault operation refused"}
                    try:
                        connection.sendall(json.dumps(result).encode() + b"\n")
                    except OSError:
                        pass
        finally:
            deadline.cancel()
            socket_path.unlink(missing_ok=True)
            grant_path.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("init", "add", "serve"):
        command = commands.add_parser(name)
        command.add_argument("--vault", required=True)
        if name == "add":
            command.add_argument("--alias", required=True)
            command.add_argument("--origin", required=True)
            command.add_argument("--methods", required=True)
            command.add_argument("--prefix", required=True)
        if name == "serve":
            command.add_argument("--scope", required=True)
            command.add_argument("--socket", required=True)
            command.add_argument("--grant-file", required=True)
            command.add_argument("--socket-group")
            command.add_argument("--ttl", type=int, required=True)
    args = parser.parse_args()
    if sys.platform != "linux":
        parser.error("the initial broker supports Linux only")
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    if ctypes.CDLL(None).prctl(4, 0, 0, 0, 0) != 0:  # PR_SET_DUMPABLE
        parser.error("could not disable process inspection")
    os.umask(0o077)
    try:
        if args.command == "serve" and not 1 <= args.ttl <= 86400:
            raise Refused()
        # Refuse redirected stdin: all secrets must be entered by a human TTY.
        if not sys.stdin.isatty():
            raise Refused()
        password = getpass.getpass("Vault password: ")
        if not password:
            raise Refused()
        if args.command == "init":
            if password != getpass.getpass("Confirm vault password: "):
                raise Refused()
            write_private(args.vault, encrypt({}, password))
        elif args.command == "add":
            raw = read_private(args.vault)
            credentials = decrypt(raw, password)
            credential = {"origin": args.origin, "methods": args.methods.split(","), "prefix": args.prefix,
                          "token": getpass.getpass("Bearer token: ")}
            validate_credential(args.alias, credential)
            credentials[args.alias] = credential
            path = private_path(args.vault)
            temporary = path.with_name(path.name + "." + secrets.token_hex(8))
            write_private(temporary, encrypt(credentials, password, salt=raw[4:20]))
            os.replace(temporary, path)
        else:
            raw = read_private(args.vault)
            key = file_key(raw, password)
            credentials = decrypt_with_key(raw, key)
            del password
            serve(args, key, credentials)
    except Exception:
        print("Vault operation refused. Check configuration, password, and private paths.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
