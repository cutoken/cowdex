"""Build the self-contained vault for the package's OS and architecture."""

import os
import subprocess

from .cargo import cargo_target_dir
from .targets import REPO_ROOT, TargetSpec
from .version import read_workspace_version


def build_vault(spec: TargetSpec):
    target_os = "windows" if spec.is_windows else "linux" if spec.is_linux else "darwin"
    target_arch = "arm64" if spec.target.startswith("aarch64") else "amd64"
    output = (
        cargo_target_dir()
        / "cowdex-vault"
        / spec.target
        / f"cowdex-vault{spec.exe_suffix}"
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            "go",
            "build",
            "-trimpath",
            f"-ldflags=-s -w -X main.version={read_workspace_version()}",
            "-o",
            str(output),
            ".",
        ],
        cwd=REPO_ROOT / "cowdex-vault",
        env={
            **os.environ,
            "CGO_ENABLED": "0",
            "GOOS": target_os,
            "GOARCH": target_arch,
        },
        check=True,
    )
    return output
