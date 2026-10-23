#!/usr/bin/env python3
"""Exercise public installation, updating, tool execution, and resume on a clean host."""

import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import threading
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

REPO = "cutoken/cowdex"
INSTALL_URL = f"https://raw.githubusercontent.com/{REPO}/main/scripts/install"
ROOT = Path(__file__).resolve().parents[1]
SEED = "COWDEX_RELEASE_SEED"
TOOL = "COWDEX_RELEASE_TOOL_OK"
FINAL = "COWDEX_RELEASE_TURN_OK"
RESUME = "COWDEX_RELEASE_RESUME_OK"


def run(args, env, cwd, timeout=180):
    result = subprocess.run(
        [str(arg) for arg in args],
        env=env,
        cwd=cwd,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
        stdin=subprocess.DEVNULL,
    )
    if result.returncode:
        raise RuntimeError(
            f"Command failed ({result.returncode}): {args[0]}\n{result.stdout}\n{result.stderr}"
        )
    return result.stdout


class MockModel(ThreadingHTTPServer):
    def __init__(self):
        super().__init__(("127.0.0.1", 0), ModelRequest)
        self.requests = []
        self.errors = []


class ModelRequest(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        try:
            if not self.path.endswith("/responses"):
                raise AssertionError(f"Unexpected model route: {self.path}")
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            self.server.requests.append(request)
            index = len(self.server.requests)
            if index == 1:
                code = (
                    'const result = await tools.exec_command({cmd: "echo '
                    + TOOL
                    + '", max_output_tokens: 100}); text(result);'
                )
                item = {
                    "type": "custom_tool_call",
                    "call_id": "release-tool",
                    "name": "exec",
                    "input": code,
                }
            else:
                contents = json.dumps(request.get("input", []))
                if index == 2:
                    outputs = [
                        item
                        for item in request["input"]
                        if item.get("type") == "custom_tool_call_output"
                    ]
                    assert any(TOOL in json.dumps(output) for output in outputs), (
                        "Shell tool did not return its marker"
                    )
                    text = FINAL
                elif index == 3:
                    assert (
                        SEED in contents and FINAL in contents and RESUME in contents
                    ), "Resume lost the previous conversation"
                    text = RESUME
                else:
                    raise AssertionError(f"Unexpected model request #{index}")
                item = {
                    "type": "message",
                    "role": "assistant",
                    "id": f"message-{index}",
                    "content": [{"type": "output_text", "text": text}],
                }
            events = [
                {"type": "response.created", "response": {"id": f"response-{index}"}},
                {"type": "response.output_item.done", "item": item},
                {
                    "type": "response.completed",
                    "response": {
                        "id": f"response-{index}",
                        "usage": {
                            "input_tokens": 100,
                            "output_tokens": 20,
                            "total_tokens": 120,
                        },
                    },
                },
            ]
            body = "".join(
                f"event: {event['type']}\ndata: {json.dumps(event)}\n\n"
                for event in events
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        except Exception as error:
            self.server.errors.append(str(error))
            self.send_error(400, str(error))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    options = parser.parse_args()
    report = {"os": platform.system(), "architecture": platform.machine(), "checks": []}
    options.report.parent.mkdir(parents=True, exist_ok=True)

    def check(name, action):
        print(f"Checking {name}...", flush=True)
        try:
            detail = action()
            report["checks"].append({"name": name, "passed": True, "detail": detail})
            print(f"PASS: {name}", flush=True)
            return detail
        except Exception as error:
            report["checks"].append(
                {"name": name, "passed": False, "detail": str(error)}
            )
            print(f"FAIL: {name}: {error}", flush=True)
            return None
        finally:
            options.report.write_text(json.dumps(report, indent=2) + "\n")

    with tempfile.TemporaryDirectory(
        prefix="cowdex release ", dir=Path.home()
    ) as scratch:
        root = Path(scratch)
        home = root / "home"
        home.mkdir()
        workspace = root / "project with spaces"
        workspace.mkdir()
        package_root = root / "packages"
        bin_dir = root / "bin with spaces"
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("CODEX_", "COWDEX_", "OPENAI_"))
        }
        env.update(
            CODEX_HOME=str(home),
            COWDEX_PACKAGE_ROOT=str(package_root),
            COWDEX_INSTALL_DIR=str(bin_dir),
        )
        windows = os.name == "nt"
        powershell = [
            "powershell.exe",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
        ]
        launcher = bin_dir / ("cowdex.cmd" if windows else "cowdex")
        vault_launcher = bin_dir / ("cowdex-vault.cmd" if windows else "cowdex-vault")

        def install():
            if windows:
                return run(
                    [*powershell, f"irm {INSTALL_URL}.ps1 | iex"],
                    env,
                    workspace,
                    timeout=300,
                )
            return run(
                ["sh", "-c", f"curl -fsSL {INSTALL_URL}.sh | sh"],
                env,
                workspace,
                timeout=300,
            )

        if check("documented public installer", install) is None:
            return 1
        initial = next((package_root / "releases").iterdir())
        metadata = json.loads((initial / "codex-package.json").read_text())
        report["release"] = metadata.get("releaseVersion", metadata["version"])
        executable = initial / "bin" / ("cowdex.exe" if windows else "cowdex")

        def launch():
            output = run([launcher, "--help"], env, workspace)
            assert (
                "cowdex" in output.lower()
                and "--worker" in output
                and "update" in output
            )
            return run([vault_launcher, "--version"], env, workspace)

        check("installed launchers and CLI help", launch)
        if windows:
            check(
                "Windows command discoverability after installation",
                lambda: run(
                    [
                        *powershell,
                        f"irm {INSTALL_URL}.ps1 | iex; $ErrorActionPreference='Stop'; (Get-Command cowdex -ErrorAction Stop).Source",
                    ],
                    env,
                    workspace,
                    timeout=300,
                ),
            )
            # The extra installation above also exercises replacement of existing .cmd launchers.
        env["PATH"] = str(bin_dir) + os.pathsep + env.get("PATH", "")
        check(
            "bundled vault and code-mode host",
            lambda: run(
                [sys.executable, ROOT / "scripts/smoke_cowdex_package.py", initial],
                env,
                workspace,
            ),
        )

        with MockModel() as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            config = home / "config.toml"
            config.write_text(f"""model = "gpt-6.1-sol"
model_provider = "release_test"
check_for_update_on_startup = false
[features]
code_mode = true
enable_request_compression = false
apps = false
[model_providers.release_test]
name = "Release test"
base_url = "http://127.0.0.1:{server.server_port}/v1"
wire_api = "responses"
supports_websockets = false
""")
            auth = home / "auth.json"
            auth.write_text('{"OPENAI_API_KEY":"release-test-placeholder"}\n')
            base = ["--yolo", "exec", "--skip-git-repo-check", "--json"]
            session = []

            def conversation():
                stdout = run([launcher, *base, SEED], env, workspace)
                assert FINAL in stdout, stdout
                events = [
                    json.loads(line)
                    for line in stdout.splitlines()
                    if line.startswith("{")
                ]
                session.extend(
                    event["thread_id"]
                    for event in events
                    if event.get("type") == "thread.started"
                )
                assert len(session) == 1, stdout
                assert len(server.requests) == 2 and not server.errors, server.errors
                return (
                    "Mock model -> bundled V8 host -> real shell command -> final reply"
                )

            check("conversation and real tool execution", conversation)
            preserved = {path: path.read_bytes() for path in [config, auth]}
            before = set((package_root / "releases").iterdir())

            def update():
                stdout = run([launcher, "update"], env, workspace, timeout=300)
                added = set((package_root / "releases").iterdir()) - before
                assert len(added) == 1, stdout
                updated = added.pop()
                assert all(
                    path.read_bytes() == content for path, content in preserved.items()
                ), "Update changed user settings or credentials"
                assert initial.is_dir(), "Update removed the old package"
                assert run([executable, "--version"], env, workspace), (
                    "Old executable stopped working"
                )
                assert run([launcher, "--version"], env, workspace), (
                    "Updated launcher stopped working"
                )
                current = json.loads((updated / "codex-package.json").read_text())
                assert (
                    current.get("releaseVersion", current["version"])
                    == report["release"]
                )
                return "Public cowdex update replaced launchers and preserved settings, credentials, old package, and session"

            check("public updater with an existing session", update)

            def resume():
                assert session, "Initial conversation did not create a session"
                stdout = run(
                    [launcher, *base, "resume", session[0], RESUME], env, workspace
                )
                assert RESUME in stdout, stdout
                assert len(server.requests) == 3 and not server.errors, server.errors
                assert len(list((home / "sessions").rglob("rollout-*.jsonl"))) == 1
                return "Same session resumed after update with the original prompt and reply"

            check("resume across update", resume)
            server.shutdown()
            thread.join(timeout=5)

        def advertised_usage_export():
            stdout = run([launcher, "--help"], env, workspace)
            assert "--export-usage" in stdout, (
                "README advertises --export-usage, but the published binary does not implement it"
            )
            output = workspace / "usage.png"
            run([launcher, "--export-usage", "--usage-output", output], env, workspace)
            assert output.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
            return "Documented usage export produced a PNG"

        check("documented usage export", advertised_usage_export)
        report["passed"] = all(item["passed"] for item in report["checks"])
        report["upgrade_scope"] = (
            "Reinstall published latest through the real updater; older release binaries are unavailable"
        )
        options.report.write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report, indent=2))
        return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
