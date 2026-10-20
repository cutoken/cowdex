#!/usr/bin/env python3
"""Check a staged CLI and execute JavaScript in its bundled V8 host, offline."""

import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
from threading import Timer
import urllib.request


def main():
    package = Path(sys.argv[1]).resolve()
    suffix = ".exe" if os.name == "nt" else ""
    subprocess.run([str(package / f"bin/cowdex{suffix}"), "--version"], check=True)
    subprocess.run(
        [str(package / f"bin/cowdex-vault{suffix}"), "--version"], check=True
    )
    with tempfile.TemporaryDirectory(prefix="cowdex-vault-smoke-") as vault_home:
        with subprocess.Popen(
            [
                str(package / f"bin/cowdex-vault{suffix}"),
                "--serve",
                "--port",
                "0",
                "--ttl",
                "60",
            ],
            env={**os.environ, "COWDEX_VAULT_HOME": vault_home},
            stdout=subprocess.PIPE,
            text=True,
        ) as vault:
            timeout = Timer(30, vault.kill)
            timeout.start()
            try:
                address = (
                    vault.stdout.readline().strip().removeprefix("Vault manager: ")
                )
                if not address.startswith("http://127.0.0.1:"):
                    raise RuntimeError("Bundled vault manager did not start")

                def api(path, body, token=""):
                    request = urllib.request.Request(
                        address + "/api/" + path,
                        data=json.dumps(body).encode(),
                        headers={
                            "Origin": address,
                            "Content-Type": "application/json",
                            "Authorization": "Bearer " + token,
                        },
                    )
                    with urllib.request.urlopen(request, timeout=5) as response:
                        return json.load(response)

                unlocked = api(
                    "unlock",
                    {
                        "password": "fake-smoke-password",
                        "confirmPassword": "fake-smoke-password",
                    },
                )
                token = unlocked["adminToken"]
                api(
                    "save",
                    {
                        "alias": "test",
                        "template": "cloudflare",
                        "token": "fake-smoke-key",
                        "prefix": "/",
                        "methods": ["GET"],
                    },
                    token,
                )
                profile = json.loads((Path(vault_home) / "grant.json").read_text())

                def broker(request):
                    if profile["socket"].startswith("tcp://"):
                        host, port = profile["socket"].removeprefix("tcp://").split(":")
                        connection = socket.create_connection(
                            (host, int(port)), timeout=5
                        )
                    else:
                        connection = socket.socket(socket.AF_UNIX)
                        connection.settimeout(5)
                        connection.connect(profile["socket"])
                    with connection:
                        connection.sendall(json.dumps(request).encode() + b"\n")
                        with connection.makefile("rb") as response:
                            return json.loads(response.readline(65537))

                pending = broker(
                    {"op": "attach", "ticket": profile["ticket"], "scope": ["test"]}
                )
                api("approve", {"code": pending["approval_code"]}, token)
                attached = broker(
                    {"op": "claim", "claim_token": pending["claim_token"]}
                )
                assert broker({"op": "status", "session": attached["session"]})[
                    "aliases"
                ] == ["test"]
                api("lock", {}, token)
                if vault.wait(timeout=10) != 0:
                    raise RuntimeError("Bundled vault manager failed to lock")
            finally:
                timeout.cancel()
                if vault.poll() is None:
                    vault.terminate()
                    vault.wait(timeout=5)
    with subprocess.Popen(
        [str(package / f"bin/codex-code-mode-host{suffix}"), "--listen", "stdio"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
    ) as host:
        timeout = Timer(30, host.kill)
        timeout.start()
        try:

            def exchange(message):
                payload = json.dumps(message).encode()
                host.stdin.write(struct.pack("<I", len(payload)) + payload)
                host.stdin.flush()

            def receive():
                prefix = host.stdout.read(4)
                if len(prefix) != 4:
                    raise RuntimeError("Bundled code-mode host closed before replying")
                size = struct.unpack("<I", prefix)[0]
                return json.loads(host.stdout.read(size))

            exchange(
                {
                    "type": "connection/hello",
                    "supportedVersions": [1],
                    "requiredCapabilities": [],
                    "optionalCapabilities": [],
                }
            )
            assert receive()["type"] == "connection/ready"
            exchange(
                {
                    "type": "operation/request",
                    "id": 1,
                    "request": {"method": "session/open", "sessionId": "package-smoke"},
                }
            )
            assert receive()["result"]["status"] == "ok"
            exchange(
                {
                    "type": "operation/request",
                    "id": 2,
                    "request": {
                        "method": "session/execute",
                        "sessionId": "package-smoke",
                        "request": {
                            "tool_call_id": "smoke",
                            "enabled_tools": [],
                            "source": 'text("COWDEX_PACKAGE_OK")',
                            "yield_time_ms": 1000,
                            "max_output_tokens": 100,
                        },
                    },
                }
            )
            while True:
                message = receive()
                if message["type"] == "execute/initialResponse":
                    assert message["result"]["status"] == "ok", message
                    assert "COWDEX_PACKAGE_OK" in json.dumps(message), message
                    break
            host.stdin.close()
            assert host.wait(timeout=10) == 0
        finally:
            timeout.cancel()
    print("Bundled CLI and V8 host smoke check passed")


if __name__ == "__main__":
    main()
