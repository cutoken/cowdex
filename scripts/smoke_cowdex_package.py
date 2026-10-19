#!/usr/bin/env python3
"""Check a staged CLI and execute JavaScript in its bundled V8 host, offline."""

import json
import os
from pathlib import Path
import struct
import subprocess
import sys
from threading import Timer


def main():
    package = Path(sys.argv[1]).resolve()
    suffix = ".exe" if os.name == "nt" else ""
    subprocess.run([str(package / f"bin/cowdex{suffix}"), "--version"], check=True)
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
