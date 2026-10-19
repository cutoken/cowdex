import base64
import copy
import json
import multiprocessing
import os
from pathlib import Path
import socket
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from cryptography.exceptions import InvalidTag

import broker


CREDENTIAL = {
    "origin": "https://api.example.com",
    "methods": ["GET", "POST"],
    "prefix": "/accounts/one",
    "token": "FAKE_CREDENTIAL_ONLY",
}


class BrokerTests(unittest.TestCase):
    def setUp(self):
        self.vault = broker.Broker({"cf": copy.deepcopy(CREDENTIAL)}, "fake-ticket", int(time.time()) + 60)

    def attach(self):
        return self.vault.handle({"op": "attach", "ticket": "fake-ticket", "scope": ["cf"]})["session"]

    def test_encryption_wrong_password_and_tamper(self):
        credentials = {"cf": CREDENTIAL}
        raw = broker.encrypt(credentials, "fake-password")
        self.assertNotIn(CREDENTIAL["token"].encode(), raw)
        self.assertEqual(broker.decrypt(raw, "fake-password"), credentials)
        for payload, password in [(raw, "wrong"), (raw[:-1] + bytes([raw[-1] ^ 1]), "fake-password")]:
            with self.assertRaises(InvalidTag):
                broker.decrypt(payload, password)

    def test_scope_mismatch_preserves_ticket_then_single_use(self):
        for scope in (["other"], ["cf", "cf"], []):
            with self.assertRaises(broker.Refused):
                self.vault.handle({"op": "attach", "ticket": "fake-ticket", "scope": scope})
        session = self.attach()
        with self.assertRaises(broker.Refused):
            self.attach()
        self.assertEqual(self.vault.handle({"op": "status", "session": session}),
                         {"aliases": ["cf"], "expires_at": self.vault.expires_at})
        self.assertEqual(self.vault.handle({"op": "detach", "session": session}), {})
        with self.assertRaises(broker.Refused):
            self.vault.handle({"op": "status", "session": session})

    @patch("broker.upstream")
    def test_rotation_reuses_key_and_freezes_scope_policy(self, send):
        raw = broker.encrypt({"cf": CREDENTIAL}, "fake-password")
        key = broker.file_key(raw, "fake-password")
        replacement = {**CREDENTIAL, "token": "FAKE_REPLACEMENT_ONLY", "prefix": "/"}
        rotated = broker.encrypt({"cf": replacement, "other": replacement}, "fake-password", salt=raw[4:20])
        self.assertEqual(broker.decrypt_with_key(rotated, key), {"cf": replacement, "other": replacement})
        self.vault.reload_credentials = lambda: {"cf": broker.decrypt_with_key(rotated, key)["cf"]}
        session = self.attach()
        request = {"op": "request", "session": session, "credential": "cf", "method": "GET",
                   "path": "/accounts/one/zones"}
        self.vault.handle(request)
        send.assert_called_once_with({**CREDENTIAL, "token": replacement["token"]}, "GET", request["path"], None)
        with self.assertRaises(broker.Refused):
            self.vault.handle({**request, "path": "/accounts/two"})
        with self.assertRaises(broker.Refused):
            self.vault.handle({**request, "credential": "other"})
        send.reset_mock()
        repurposed = {**replacement, "origin": "https://different.example.com", "token": "FAKE_DIFFERENT_ORIGIN_TOKEN"}
        rotated = broker.encrypt({"cf": repurposed}, "fake-password", salt=raw[4:20])
        with self.assertRaises(broker.Refused):
            self.vault.handle(request)
        send.assert_not_called()
        self.assertEqual(self.vault.credentials, {"cf": {**CREDENTIAL, "token": replacement["token"]}})

    def test_expiry_and_bad_session(self):
        session = self.attach()
        with self.assertRaises(broker.Refused):
            self.vault.handle({"op": "status", "session": "invalid"})
        with patch("broker.time.time", return_value=self.vault.expires_at):
            with self.assertRaises(broker.Refused):
                self.vault.handle({"op": "status", "session": session})

    @patch("broker.upstream")
    def test_request_authorization_boundaries(self, send):
        session = self.attach()
        request = {"op": "request", "session": session, "credential": "cf", "method": "GET",
                   "path": "/accounts/one/zones?name=example%2Ecom"}
        send.return_value = {"status": 200, "body": "ok", "truncated": False}
        self.assertEqual(self.vault.handle(request), send.return_value)
        send.assert_called_once_with(CREDENTIAL, "GET", request["path"], None)
        send.reset_mock()
        for changes in [
            {"credential": "other"}, {"method": "DELETE"}, {"body": {}}, {"headers": {}},
            *({"path": path} for path in ["https://evil.test", "//evil.test", "/accounts/oneevil",
                "/accounts/one/../two", "/accounts/one/%2e%2e/two", "/accounts/one\\two",
                "/accounts//one", "/accounts/one#fragment", "/accounts/one\nInjected:yes"]),
        ]:
            with self.subTest(changes=changes), self.assertRaises(broker.Refused):
                self.vault.handle({**request, **changes})
        send.assert_not_called()

    @patch("broker.http.client.HTTPSConnection")
    def test_echo_redaction_and_upstream_contract(self, connection):
        response = connection.return_value.getresponse.return_value
        response.status = 200
        response.getheader.return_value = "identity"
        response.read.return_value = json.dumps({"echo": "Bearer " + CREDENTIAL["token"]}).encode()
        self.assertEqual(broker.upstream(CREDENTIAL, "GET", "/accounts/one", None),
                         {"status": 200, "body": '{"echo": "Bearer [REDACTED]"}', "truncated": False})
        connection.return_value.request.assert_called_once_with(
            "GET", "/accounts/one", body=None,
            headers={"Authorization": "Bearer " + CREDENTIAL["token"], "Accept-Encoding": "identity",
                     "User-Agent": "Cowdex-Vault"})
        connection.return_value.close.assert_called_once()

    @patch("broker.http.client.HTTPSConnection")
    def test_basic_form_authentication_and_echo_redaction(self, connection):
        credential = {**CREDENTIAL, "auth": "basic", "username": "FAKE_USERNAME", "body_format": "form"}
        encoded = base64.b64encode((credential["username"] + ":" + credential["token"]).encode()).decode()
        response = connection.return_value.getresponse.return_value
        response.status, response.getheader.return_value = 201, "identity"
        response.read.return_value = ("Basic " + encoded + " " + credential["token"]).encode()
        body = json.dumps({"To": "+12345", "Body": "hello & goodbye", "Tag": ["a", "b"], "Enabled": True})
        self.assertEqual(broker.upstream(credential, "POST", "/accounts/one", body),
                         {"status": 201, "body": "Basic [REDACTED] [REDACTED]", "truncated": False})
        connection.return_value.request.assert_called_once_with(
            "POST", "/accounts/one", body=b"To=%2B12345&Body=hello+%26+goodbye&Tag=a&Tag=b&Enabled=true",
            headers={"Authorization": "Basic " + encoded, "Accept-Encoding": "identity",
                     "User-Agent": "Cowdex-Vault", "Content-Type": "application/x-www-form-urlencoded"})
        connection.return_value.close.assert_called_once()

    @patch("broker.http.client.HTTPSConnection")
    @patch("broker.secrets.token_hex", return_value="a" * 48)
    def test_mailgun_basic_multipart_authentication(self, _token_hex, connection):
        credential = {**CREDENTIAL, "auth": "basic", "username": "api",
                      "body_format": "multipart"}
        encoded = base64.b64encode(("api:" + credential["token"]).encode()).decode()
        response = connection.return_value.getresponse.return_value
        response.status, response.getheader.return_value = 200, "identity"
        response.read.return_value = b'{"id":"queued"}'
        body = json.dumps({"from": "agent@example.test", "to": ["one@example.test", "two@example.test"],
                           "subject": "hello"})
        boundary = "cowdex-" + "a" * 48
        expected_body = (
            f'--{boundary}\r\nContent-Disposition: form-data; name="from"\r\n\r\nagent@example.test\r\n'
            f'--{boundary}\r\nContent-Disposition: form-data; name="to"\r\n\r\none@example.test\r\n'
            f'--{boundary}\r\nContent-Disposition: form-data; name="to"\r\n\r\ntwo@example.test\r\n'
            f'--{boundary}\r\nContent-Disposition: form-data; name="subject"\r\n\r\nhello\r\n'
            f'--{boundary}--\r\n').encode()

        self.assertEqual(broker.upstream(credential, "POST", "/v3/example.test/messages", body),
                         {"status": 200, "body": '{"id":"queued"}', "truncated": False})
        connection.return_value.request.assert_called_once_with(
            "POST", "/v3/example.test/messages", body=expected_body,
            headers={"Authorization": "Basic " + encoded, "Accept-Encoding": "identity",
                     "User-Agent": "Cowdex-Vault",
                     "Content-Type": "multipart/form-data; boundary=" + boundary})

    @patch("broker.http.client.HTTPSConnection")
    def test_helio_authentication_overwrites_api_key_and_redacts_both_keys(self, connection):
        credential = {**CREDENTIAL, "auth": "helio", "username": "FAKE_PUBLIC_API_KEY",
                      "token": "FAKE_SECRET_API_KEY", "body_format": "json"}
        response = connection.return_value.getresponse.return_value
        response.status, response.getheader.return_value = 200, "identity"
        response.read.return_value = b"FAKE_SECRET_API_KEY FAKE_PUBLIC_API_KEY"
        body = '{"name":"agent checkout"}'

        self.assertEqual(broker.upstream(
            credential, "POST", "/v1/paylink?status=active&apiKey=ATTACKER&empty=", body),
            {"status": 200, "body": "[REDACTED] [REDACTED]", "truncated": False})
        connection.return_value.request.assert_called_once_with(
            "POST", "/v1/paylink?status=active&empty=&apiKey=FAKE_PUBLIC_API_KEY",
            body=body.encode(),
            headers={"Authorization": "Bearer FAKE_SECRET_API_KEY", "Accept-Encoding": "identity",
                     "User-Agent": "Cowdex-Vault", "Content-Type": "application/json"})

    @patch("broker.http.client.HTTPSConnection")
    def test_invalid_auth_and_form_bodies_never_send(self, connection):
        for extra in [{"auth": "unknown"}, {"auth": "basic"}, {"auth": "helio"},
                      {"auth": "basic", "username": "a:b"},
                      {"auth": "basic", "username": "bad\r\nheader"}, {"body_format": "unknown"}]:
            with self.subTest(extra=extra), self.assertRaises(broker.Refused):
                broker.upstream({**CREDENTIAL, **extra}, "GET", "/accounts/one", None)
        for body in ['not json', '[]', '{"a": {"nested": "value"}}', '{"a": [[1]]}',
                     '{"a": null}', '{"a": NaN}']:
            with self.subTest(body=body), self.assertRaises(broker.Refused):
                broker.upstream({**CREDENTIAL, "body_format": "form"}, "POST", "/accounts/one", body)
        connection.return_value.request.assert_not_called()

    @patch("broker.upstream")
    def test_auth_policy_changes_refuse_rotation(self, send):
        credential = {**CREDENTIAL, "auth": "basic", "username": "FAKE_USERNAME", "body_format": "form"}
        self.vault.credentials = {"cf": copy.deepcopy(credential)}
        session = self.attach()
        request = {"op": "request", "session": session, "credential": "cf", "method": "POST",
                   "path": "/accounts/one", "body": '{"text":"hello"}'}
        for changes in [{"auth": "bearer"}, {"username": "DIFFERENT_USERNAME"}, {"body_format": "json"}]:
            self.vault.reload_credentials = lambda: {"cf": {**credential, **changes, "token": "FAKE_ROTATED"}}
            with self.subTest(changes=changes), self.assertRaises(broker.Refused):
                self.vault.handle(request)
        send.assert_not_called()
        self.assertEqual(self.vault.credentials, {"cf": credential})
        replacement = {**credential, "token": "FAKE_ROTATED"}
        self.vault.reload_credentials = lambda: {"cf": replacement}
        self.vault.handle(request)
        send.assert_called_once_with(replacement, "POST", "/accounts/one", request["body"])

    @patch("broker.http.client.HTTPSConnection")
    def test_redirect_encoding_and_oversize_rejected(self, connection):
        response = connection.return_value.getresponse.return_value
        for status, encoding, payload in [(302, "identity", b""), (200, "gzip", b""),
                                         (200, "identity", b"a" * (broker.MAX_RESPONSE + 1))]:
            response.status, response.getheader.return_value, response.read.return_value = status, encoding, payload
            with self.subTest(status=status, encoding=encoding), self.assertRaises(broker.Refused):
                broker.upstream(CREDENTIAL, "GET", "/accounts/one", None)

    def test_private_file_creation_and_symlink_rejection(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "vault"
            broker.write_private(path, b"fake")
            self.assertEqual(broker.read_private(path), b"fake")
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(FileExistsError):
                broker.write_private(path, b"overwrite")
            link = Path(directory) / "link"
            link.symlink_to(path)
            with self.assertRaises(OSError):
                broker.read_private(link)

    def test_private_path_accepts_only_user_private_group_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory) / "private-group"
            parent.mkdir(mode=0o700)
            parent.chmod(0o770)
            path = parent / "vault.enc"
            owner = SimpleNamespace(pw_name="owner", pw_uid=os.getuid(), pw_gid=os.getgid())
            private_group = SimpleNamespace(gr_name="owner", gr_mem=[])
            patches = (
                patch("broker.pwd.getpwuid", return_value=owner),
                patch("broker.grp.getgrgid", return_value=private_group),
                patch("broker.pwd.getpwall", return_value=[owner]),
            )
            with patches[0], patches[1], patches[2]:
                self.assertEqual(broker.private_path(path), path)

            with patch("broker.pwd.getpwuid", return_value=owner), \
                    patch("broker.grp.getgrgid", return_value=private_group), \
                    patch("broker.pwd.getpwall", return_value=[owner]), \
                    patch("broker.os.getxattr", return_value=b"named-user-acl"), \
                    self.assertRaises(broker.Refused):
                broker.private_path(path)

            rejected_accounts = [
                (SimpleNamespace(gr_name="owner", gr_mem=["other"]), [owner]),
                (private_group, [owner, SimpleNamespace(pw_name="other", pw_uid=os.getuid() + 1,
                                                        pw_gid=os.getgid())]),
                (SimpleNamespace(gr_name="shared", gr_mem=[]), [owner]),
            ]
            for group, accounts in rejected_accounts:
                with self.subTest(group=group, accounts=accounts), \
                        patch("broker.pwd.getpwuid", return_value=owner), \
                        patch("broker.grp.getgrgid", return_value=group), \
                        patch("broker.pwd.getpwall", return_value=accounts), \
                        self.assertRaises(broker.Refused):
                    broker.private_path(path)

            parent.chmod(0o777)
            with self.assertRaises(broker.Refused):
                broker.private_path(path)

            parent.chmod(0o770)
            original_lstat = Path.lstat

            def nonowner_lstat(candidate):
                info = original_lstat(candidate)
                if candidate == parent:
                    return SimpleNamespace(st_mode=info.st_mode, st_uid=os.getuid() + 1,
                                           st_gid=info.st_gid)
                return info

            with patch.object(Path, "lstat", nonowner_lstat), self.assertRaises(broker.Refused):
                broker.private_path(path)

    def test_socket_single_use_detach_and_process_deadline(self):
        for detach in (True, False):
            with self.subTest(detach=detach), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                args = SimpleNamespace(vault=str(root / "vault"), socket=str(root / "broker.sock"),
                                       grant_file=str(root / "grant.json"), scope="cf", socket_group=None,
                                       ttl=3 if detach else 1)
                raw = broker.encrypt({"cf": CREDENTIAL}, "fake-password")
                broker.write_private(args.vault, raw)
                process = multiprocessing.Process(target=broker.serve,
                    args=(args, broker.file_key(raw, "fake-password"), {"cf": copy.deepcopy(CREDENTIAL)}))
                process.start()
                try:
                    deadline = time.monotonic() + 2
                    while not Path(args.grant_file).exists() and time.monotonic() < deadline:
                        time.sleep(0.01)
                    grant = json.loads(Path(args.grant_file).read_text())
                    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
                        client.connect(args.socket)
                        client.sendall(json.dumps({"op": "attach", "ticket": grant["ticket"], "scope": ["cf"]}).encode() + b"\n")
                        result = json.loads(client.makefile("rb").readline())
                    self.assertEqual(result["aliases"], ["cf"])
                    if detach:
                        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
                            client.connect(args.socket)
                            client.sendall(json.dumps({"op": "detach", "session": result["session"]}).encode() + b"\n")
                            self.assertEqual(json.loads(client.makefile("rb").readline()), {})
                    process.join(4)
                    self.assertEqual(process.exitcode, 0)
                    self.assertFalse(Path(args.socket).exists())
                    self.assertFalse(Path(args.grant_file).exists())
                finally:
                    if process.is_alive():
                        process.terminate()
                        process.join()


if __name__ == "__main__":
    unittest.main()
