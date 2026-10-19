import http.client
import hashlib
import hmac
import json
from pathlib import Path
import socket
import socketserver
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import broker
import web


class WebTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.stopped = []
        self.stop_event = threading.Event()

        def stopped(code):
            self.stopped.append(code)
            self.stop_event.set()

        self.manager = web.Manager(Path(self.directory.name), 7200, stopped)
        self.http = web.ThreadingHTTPServer(("127.0.0.1", 0), web.HttpHandler)
        self.http.manager = self.manager
        self.http.proof = b"fake-service-proof"
        self.http.authority = f"127.0.0.1:{self.http.server_port}"
        self.unix = socketserver.ThreadingUnixStreamServer(str(self.manager.socket), web.SocketHandler)
        self.unix.manager = self.manager
        self.threads = [threading.Thread(target=server.serve_forever) for server in (self.http, self.unix)]
        for thread in self.threads:
            thread.start()
        self.admin_token = ""

    def tearDown(self):
        for server in (self.http, self.unix):
            server.shutdown()
            server.server_close()
        for thread in self.threads:
            thread.join()
        self.directory.cleanup()

    def request(self, path, body=None, headers=None):
        connection = http.client.HTTPConnection("127.0.0.1", self.http.server_port, timeout=5)
        fields = {"Origin": "http://" + self.http.authority, "Content-Type": "application/json",
                  "Authorization": "Bearer " + self.admin_token}
        fields.update(headers or {})
        connection.request("GET" if body is None else "POST", path,
                           body=None if body is None else json.dumps(body), headers=fields)
        response = connection.getresponse()
        result = (response.status, json.loads(response.read()))
        self.assertIsNone(response.getheader("Set-Cookie"))
        if "adminToken" in result[1]:
            self.admin_token = result[1]["adminToken"]
        connection.close()
        return result

    def unlock(self):
        status, state = self.request("/api/unlock", {"password": "fake password", "confirmPassword": "fake password"})
        self.assertEqual(status, 200)
        self.assertTrue(state["unlocked"])
        return state

    def save(self, alias="cf", token="FAKE_PROVIDER_TOKEN", **changes):
        body = {"alias": alias, "template": "custom", "origin": "https://api.example.com",
                "prefix": "/zones", "methods": ["GET"], "auth": "bearer",
                "body_format": "json", "token": token}
        body.update(changes)
        return self.request("/api/save", body)

    def socket_request(self, message):
        with socket.socket(socket.AF_UNIX) as connection:
            connection.connect(str(self.manager.socket))
            connection.sendall(json.dumps(message).encode() + b"\n")
            with connection.makefile("rb") as stream:
                return json.loads(stream.readline())

    def begin_attach(self, scope, ticket=None):
        if ticket is None:
            ticket = json.loads(broker.read_private(self.manager.profile))["ticket"]
        return self.socket_request({"op": "attach", "ticket": ticket, "scope": scope})

    def approve(self, pending):
        status, result = self.request("/api/approve", {"code": pending["approval_code"]})
        self.assertEqual(status, 200)
        return result

    def claim(self, pending):
        return self.socket_request({"op": "claim", "claim_token": pending["claim_token"]})

    def attach(self, scope, ticket=None):
        pending = self.begin_attach(scope, ticket)
        self.approve(pending)
        return self.claim(pending)

    def test_password_metadata_encrypted_save_and_reopen(self):
        status, locked = self.request("/api/state")
        self.assertEqual(status, 200)
        self.assertEqual({key: locked[key] for key in ("unlocked", "exists")},
                         {"unlocked": False, "exists": False})
        self.assertIn("cloudflare", {template["id"] for template in locked["templates"]})
        initial = self.unlock()
        status, state = self.save()
        self.assertEqual(status, 200)
        self.assertEqual(state["credentials"], [{
            "alias": "cf", "origin": "https://api.example.com", "prefix": "/zones",
            "methods": ["GET"], "template": "custom", "auth": "bearer",
            "body_format": "json", "hasUsername": False,
        }])
        self.assertEqual(state["expiresAt"], initial["expiresAt"])
        self.assertTrue(state["ready"])
        self.assertNotIn("token", state["credentials"][0])
        self.assertNotIn("username", state["credentials"][0])
        self.assertNotIn(b"FAKE_PROVIDER_TOKEN", self.manager.vault.read_bytes())
        self.assertEqual(self.save(token="")[0], 200)
        stored = broker.decrypt(broker.read_private(self.manager.vault), "fake password")
        self.assertEqual(stored["cf"]["token"], "FAKE_PROVIDER_TOKEN")
        reopened = web.Manager(Path(self.directory.name), 7200, self.stopped.append)
        self.assertEqual(reopened.unlock({"password": "fake password"})["credentials"], state["credentials"])
        self.manager.next_attempt = 0
        self.assertEqual(self.request("/api/unlock", {"password": "wrong"})[0], 403)
        self.assertEqual(self.manager.expires_at, initial["expiresAt"])
        self.manager.next_attempt = 0
        self.assertEqual(self.request("/api/unlock", {"password": "fake password"})[1]["expiresAt"], initial["expiresAt"])

    def test_browser_authentication_and_origin_boundaries(self):
        challenge = "a" * 64
        self.assertEqual(self.request("/api/proof?challenge=" + challenge),
                         (200, {"proof": hmac.new(self.http.proof, challenge.encode(), hashlib.sha256).hexdigest()}))
        self.assertEqual(self.save()[0], 403)
        self.assertEqual(self.request("/api/unlock", {"password": "x", "confirmPassword": "x"},
                                      {"Origin": "https://attacker.example"})[0], 403)
        self.assertEqual(self.request("/api/state", headers={"Host": "attacker.example"})[0], 403)
        self.unlock()
        self.save()
        self.admin_token = "wrong"
        status, state = self.request("/api/state")
        self.assertEqual(status, 200)
        self.assertEqual({key: state[key] for key in ("unlocked", "exists")},
                         {"unlocked": False, "exists": True})
        self.assertEqual(self.request("/api/lock", {})[0], 403)

    def test_scope_ticket_rotation_token_rotation_and_detach(self):
        self.unlock()
        self.save()
        self.save(alias="github")
        self.save(alias="private", shared=False)
        profile = json.loads(broker.read_private(self.manager.profile))
        self.assertEqual(profile["scope"], ["cf", "github", "private"])
        request = {"op": "attach", "ticket": profile["ticket"], "scope": ["private"]}
        pending = self.socket_request(request)
        self.approve(pending)
        private = self.claim(pending)
        self.assertEqual(private["aliases"], ["private"])
        profile = json.loads(broker.read_private(self.manager.profile))
        request["scope"] = ["cf"]
        request["ticket"] = profile["ticket"]
        pending = self.socket_request(request)
        self.approve(pending)
        attached = self.claim(pending)
        self.assertEqual(attached["aliases"], ["cf"])
        self.assertIn("error", self.socket_request(request))
        rotated = json.loads(broker.read_private(self.manager.profile))
        self.assertNotEqual(rotated["ticket"], profile["ticket"])
        other = self.attach(["github"], rotated["ticket"])
        call = {"op": "request", "session": attached["session"], "credential": "github",
                "method": "GET", "path": "/zones", "body": None}
        self.assertIn("error", self.socket_request(call))
        self.save(token="FAKE_ROTATED_TOKEN")
        call["credential"] = "cf"
        with patch("broker.upstream", return_value={"status": 200, "body": "ok", "truncated": False}) as upstream:
            self.assertEqual(self.socket_request(call), {"status": 200, "body": "ok", "truncated": False})
            self.assertEqual(upstream.call_args.args[0]["token"], "FAKE_ROTATED_TOKEN")
        self.assertEqual(self.socket_request({"op": "detach", "session": attached["session"]}), {})
        self.assertIn("error", self.socket_request(call))
        self.assertEqual(self.socket_request({"op": "status", "session": other["session"]})["aliases"], ["github"])
        self.save(alias="github", token="", prefix="/repos")
        self.assertIn("error", self.socket_request({"op": "status", "session": other["session"]}))

    def test_template_defaults_ignore_forged_provider_policy(self):
        self.unlock()
        status, state = self.request("/api/save", {
            "alias": "cloudflare", "template": "cloudflare", "token": "FAKE_CLOUDFLARE_TOKEN",
            "origin": "https://attacker.example", "auth": "basic", "body_format": "form",
            "username": "FORGED_USERNAME", "shared": False,
        })
        self.assertEqual(status, 200)
        expected_methods = sorted(broker.METHODS)
        self.assertEqual(state["credentials"], [{
            "alias": "cloudflare", "origin": "https://api.cloudflare.com", "prefix": "/",
            "methods": expected_methods, "template": "cloudflare", "auth": "bearer",
            "body_format": "json", "hasUsername": False,
        }])
        stored = broker.decrypt(broker.read_private(self.manager.vault), "fake password")["cloudflare"]
        self.assertEqual(stored, {
            "template": "cloudflare", "shared": True, "prefix": "/", "methods": expected_methods,
            "origin": "https://api.cloudflare.com", "auth": "bearer", "body_format": "json",
            "token": "FAKE_CLOUDFLARE_TOKEN",
        })

    def test_basic_secret_retention_non_disclosure_and_provider_change(self):
        self.unlock()
        status, state = self.save(alias="twilio", template="twilio-api-key",
                                  username="FAKE_TWILIO_KEY_SID", token="FAKE_TWILIO_SECRET")
        self.assertEqual(status, 200)
        metadata = state["credentials"][0]
        self.assertEqual((metadata["template"], metadata["auth"], metadata["body_format"],
                          metadata["hasUsername"]),
                         ("twilio-api-key", "basic", "form", True))
        self.assertNotIn("token", metadata)
        self.assertNotIn("username", metadata)
        self.assertEqual(self.save(alias="twilio", template="twilio-api-key", token="",
                                   username="")[0], 200)
        stored = broker.decrypt(broker.read_private(self.manager.vault), "fake password")["twilio"]
        self.assertEqual((stored["username"], stored["token"]),
                         ("FAKE_TWILIO_KEY_SID", "FAKE_TWILIO_SECRET"))

        self.assertEqual(self.save(alias="twilio", template="cloudflare", token="")[0], 403)
        unchanged = broker.decrypt(broker.read_private(self.manager.vault), "fake password")["twilio"]
        self.assertEqual(unchanged, stored)

    def test_mailgun_template_uses_fixed_basic_username(self):
        self.unlock()
        status, state = self.save(alias="mailgun", template="mailgun-eu",
                                  username="FORGED_USERNAME", token="FAKE_MAILGUN_API_KEY")
        self.assertEqual(status, 200)
        self.assertEqual(state["credentials"], [{
            "alias": "mailgun", "origin": "https://api.eu.mailgun.net", "prefix": "/zones",
            "methods": ["GET"], "template": "mailgun-eu", "auth": "basic",
            "body_format": "multipart", "hasUsername": True,
        }])
        stored = broker.decrypt(broker.read_private(self.manager.vault), "fake password")["mailgun"]
        self.assertEqual(stored["username"], "api")
        self.assertEqual(stored["token"], "FAKE_MAILGUN_API_KEY")
        self.assertNotIn("username", state["credentials"][0])
        self.assertNotIn("token", state["credentials"][0])

    def test_policy_change_revokes_attached_session(self):
        self.unlock()
        self.save()
        profile = json.loads(broker.read_private(self.manager.profile))
        attached = self.attach(["cf"], profile["ticket"])
        self.assertEqual(self.save(token="", methods=["GET", "POST"])[0], 200)
        self.assertIn("error", self.socket_request({"op": "status", "session": attached["session"]}))

    def test_ninth_credential_is_rejected(self):
        self.unlock()
        for number in range(8):
            self.assertEqual(self.save(alias=f"key{number}")[0], 200)
        self.assertEqual(self.save(alias="key8")[0], 403)
        self.assertEqual(len(self.manager.credentials), 8)

    def test_legacy_nine_credential_vault_remains_usable(self):
        credentials = {
            f"legacy{number}": {
                "origin": "https://api.example.com", "prefix": "/zones", "methods": ["GET"],
                "token": f"FAKE_LEGACY_TOKEN_{number}", "shared": number < 8,
            }
            for number in range(9)
        }
        broker.write_private(self.manager.vault, broker.encrypt(credentials, "fake password"))
        state = self.unlock()
        self.assertEqual(len(state["credentials"]), 9)

        self.assertEqual(self.save(alias="legacy8", token="")[0], 200)
        profile = json.loads(broker.read_private(self.manager.profile))
        self.assertEqual(profile["scope"], [f"legacy{number}" for number in range(8)])
        default = self.attach(profile["scope"], profile["ticket"])
        self.assertEqual(default["aliases"], profile["scope"])
        rotated = json.loads(broker.read_private(self.manager.profile))
        ninth = self.attach(["legacy8"], rotated["ticket"])
        self.assertEqual(ninth["aliases"], ["legacy8"])

    def test_attachment_requires_authenticated_approval_and_private_claim(self):
        self.unlock()
        self.save()
        pending = self.begin_attach(["cf"])
        self.assertEqual(pending["aliases"], ["cf"])
        self.assertNotIn("session", pending)
        self.assertEqual(self.manager.sessions, {})
        self.assertEqual(self.claim(pending), pending)

        state = self.request("/api/state")[1]
        profile = json.loads(broker.read_private(self.manager.profile))
        public = state["approvals"][0]
        self.assertEqual(public["code"], pending["approval_code"])
        self.assertNotIn("claim_token", json.dumps(state))
        self.assertNotIn("session", json.dumps(state))
        self.assertNotIn("claim_token", json.dumps(profile))
        self.assertIn("error", self.socket_request(
            {"op": "claim", "claim_token": pending["approval_code"]}))
        self.assertIn("error", self.socket_request(
            {"op": "claim", "claim_token": "wrong-private-claim"}))

        admin_token = self.admin_token
        self.admin_token = "wrong"
        self.assertEqual(self.request("/api/approve", {"code": pending["approval_code"]})[0], 403)
        self.assertEqual(self.request("/api/deny", {"code": pending["approval_code"]})[0], 403)
        self.admin_token = admin_token
        self.approve(pending)
        attached = self.socket_request({"op": "claim", "claim_token": pending["claim_token"],
                                        "scope": ["cf", "forged"]})
        self.assertEqual(attached["aliases"], ["cf"])
        self.assertIn("error", self.claim(pending))

    def test_approval_lasts_24_hours_when_unlock_window_allows(self):
        self.unlock()
        self.save()
        now = int(time.time())
        self.manager.expires_at = now + 2 * 86400
        pending = self.begin_attach(["cf"])
        with patch("web.time.time", return_value=now):
            self.approve(pending)
            attached = self.claim(pending)
        self.assertEqual(attached["expires_at"], now + 86400)

    def test_approval_deadline_denial_cancel_and_expiry(self):
        self.unlock()
        self.save()
        self.manager.expires_at = int(time.time()) + 30
        pending = self.begin_attach(["cf"])
        self.approve(pending)
        approved_until = self.manager.pending[pending["claim_token"]].approved_until
        self.assertLessEqual(approved_until, int(time.time()) + 86400)
        self.assertEqual(approved_until, self.manager.expires_at)

        denied = self.begin_attach(["cf"])
        self.assertEqual(self.request("/api/deny", {"code": denied["approval_code"]})[0], 200)
        self.assertIn("error", self.claim(denied))

        cancelled = self.begin_attach(["cf"])
        self.assertEqual(self.socket_request(
            {"op": "cancel", "claim_token": cancelled["claim_token"]}), {})
        self.assertIn("error", self.claim(cancelled))

        expired = self.begin_attach(["cf"])
        with patch("web.time.time", return_value=expired["expires_at"]):
            self.assertIn("error", self.claim(expired))

    def test_pending_limit_cleanup_and_policy_invalidation(self):
        self.unlock()
        self.save()
        pending = self.begin_attach(["cf"])
        self.assertEqual(self.save(token="FAKE_ROTATED_TOKEN")[0], 200)
        self.approve(pending)
        attached = self.claim(pending)
        with patch("broker.upstream", return_value={"status": 200, "body": "ok", "truncated": False}) as upstream:
            self.socket_request({"op": "request", "session": attached["session"],
                                 "credential": "cf", "method": "GET", "path": "/zones"})
            self.assertEqual(upstream.call_args.args[0]["token"], "FAKE_ROTATED_TOKEN")

        invalidated = self.begin_attach(["cf"])
        self.approve(invalidated)
        self.assertEqual(self.save(token="", methods=["GET", "POST"])[0], 200)
        self.assertIn("error", self.claim(invalidated))

        pendings = [self.begin_attach(["cf"]) for _ in range(8)]
        self.assertIn("error", self.begin_attach(["cf"]))
        with patch("web.time.time", return_value=max(p["expires_at"] for p in pendings)):
            self.assertEqual(self.manager.approvals(), {"approvals": []})
        self.assertNotIn("error", self.begin_attach(["cf"]))

    def test_expiry_and_lock_revoke_access(self):
        self.unlock()
        self.save()
        profile = json.loads(broker.read_private(self.manager.profile))
        with patch("web.time.time", return_value=self.manager.expires_at + 1):
            self.assertFalse(self.request("/api/state")[1]["unlocked"])
            self.assertIn("error", self.socket_request({"op": "attach", "ticket": profile["ticket"], "scope": ["cf"]}))
        self.assertEqual(self.request("/api/lock", {}), (200, {"unlocked": False}))
        self.assertTrue(self.stop_event.wait(2))
        self.assertEqual(self.stopped, [0])
        self.assertFalse(self.manager.profile.exists())
        self.assertFalse(self.manager.socket.exists())


if __name__ == "__main__":
    unittest.main()
