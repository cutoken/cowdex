#!/usr/bin/env python3
"""Install a complete Cowdex release without borrowing regular Codex's helpers."""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shlex
import shutil
import stat
import subprocess
import tarfile
import tempfile
import urllib.request
import zipfile


def host_target():
    arch = {"amd64": "x86_64", "arm64": "aarch64"}.get(
        platform.machine().lower(), platform.machine().lower()
    )
    suffix = {
        "Linux": "unknown-linux-musl",
        "Darwin": "apple-darwin",
        "Windows": "pc-windows-msvc",
    }[platform.system()]
    return f"{arch}-{suffix}"


def download_release(repo, release, destination):
    if not re.fullmatch(r"[\w.-]+/[\w.-]+", repo):
        raise ValueError("--repo must be OWNER/REPOSITORY")
    tag = (
        "latest"
        if release == "latest"
        else f"tags/cowdex-v{release.removeprefix('cowdex-v')}"
    )
    request = urllib.request.Request(
        f"https://api.github.com/repos/{repo}/releases/{tag}",
        headers={"User-Agent": "cowdex-installer"},
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        metadata = json.load(response)
    extension = "zip" if os.name == "nt" else "tar.gz"
    name = f"cowdex-{host_target()}.{extension}"
    asset = next((a for a in metadata["assets"] if a["name"] == name), None)
    if asset is None:
        raise ValueError(f"Release has no {name}")
    archive = destination / name
    with urllib.request.urlopen(asset["browser_download_url"], timeout=300) as response:
        with archive.open("wb") as output:
            shutil.copyfileobj(response, output)
    return archive


def extract_archive(archive, destination):
    # Release packages contain only regular files/directories, never links.
    def output_path(name):
        path = PurePosixPath(name)
        if path.is_absolute() or ".." in path.parts or "\\" in name or ":" in name:
            raise ValueError(f"Unsafe archive path: {name}")
        return destination.joinpath(*path.parts)

    if zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as source:
            for entry in source.infolist():
                target = output_path(entry.filename)
                if stat.S_ISLNK(entry.external_attr >> 16):
                    raise ValueError("Release archives must not contain links")
                if entry.is_dir():
                    target.mkdir(parents=True, exist_ok=True)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with source.open(entry) as src, target.open("wb") as dst:
                        shutil.copyfileobj(src, dst)
                    target.chmod((entry.external_attr >> 16) & 0o777 or 0o644)
    else:
        with tarfile.open(archive, "r:gz") as source:
            for entry in source:
                target = output_path(entry.name)
                if entry.isdir():
                    target.mkdir(parents=True, exist_ok=True)
                elif entry.isfile():
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with source.extractfile(entry) as src, target.open("wb") as dst:
                        shutil.copyfileobj(src, dst)
                    target.chmod(entry.mode & 0o777)
                else:
                    raise ValueError(
                        "Release archives must contain only files/directories"
                    )


def install(package, package_root, bin_dir):
    metadata = json.loads((package / "codex-package.json").read_text())
    expected = host_target()
    compatible = {expected, expected.replace("linux-musl", "linux-gnu")}
    if metadata.get("variant") != "cowdex" or metadata.get("target") not in compatible:
        raise ValueError(f"Package must be a Cowdex release for {expected}")
    version = metadata.get("releaseVersion", metadata["version"])
    if not re.fullmatch(
        re.escape(metadata["version"]) + r"(?:-r[1-9][0-9]*)?", version
    ):
        raise ValueError("Invalid package version")
    suffix = ".exe" if os.name == "nt" else ""
    required = [
        f"bin/cowdex{suffix}",
        f"bin/codex-code-mode-host{suffix}",
        f"codex-path/rg{suffix}",
        f"bin/cowdex-vault{suffix}",
    ]
    if platform.system() == "Linux":
        required.append("codex-resources/bwrap")
    if os.name == "nt":
        required += [
            "codex-resources/codex-command-runner.exe",
            "codex-resources/codex-windows-sandbox-setup.exe",
        ]
    for name in required:
        if not (package / name).is_file():
            raise ValueError(f"Incomplete package: missing {name}")
    subprocess.run([str(package / f"bin/cowdex{suffix}"), "--version"], check=True)
    releases = package_root / "releases"
    releases.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S%f")
    installed = releases / f"{version}-{metadata['target']}-{stamp}"
    with tempfile.TemporaryDirectory(prefix=".install-", dir=releases) as tmp:
        staged = Path(tmp) / "package"
        shutil.copytree(package, staged)
        staged.rename(installed)
    if os.name != "nt":
        with tempfile.TemporaryDirectory(prefix=".current-", dir=package_root) as tmp:
            current = Path(tmp) / "current"
            current.symlink_to(installed, target_is_directory=True)
            current.replace(package_root / "current")
    bin_dir.mkdir(parents=True, exist_ok=True)
    if os.name == "nt":
        launcher = bin_dir / "cowdex.cmd"
        contents = f'@echo off\r\n"{installed / "bin/cowdex.exe"}" -c features.multi_agent_v2.preserve_fork_prefix=true %*\r\n'
    else:
        launcher = bin_dir / "cowdex"
        contents = (
            f"#!/bin/sh\nexec {shlex.quote(str(installed / 'bin/cowdex'))} "
            '-c features.multi_agent_v2.preserve_fork_prefix=true "$@"\n'
        )
    # Atomic launcher replacement leaves previous releases and running sessions intact.
    with tempfile.NamedTemporaryFile(
        mode="w", newline="", dir=bin_dir, delete=False
    ) as output:
        output.write(contents)
        temporary = Path(output.name)
    temporary.chmod(0o755)
    temporary.replace(launcher)
    vault = bin_dir / ("cowdex-vault.cmd" if os.name == "nt" else "cowdex-vault")
    contents = (
        f'@echo off\r\n"{installed / "bin/cowdex-vault.exe"}" %*\r\n'
        if os.name == "nt"
        else f'#!/bin/sh\nexec {shlex.quote(str(installed / "bin/cowdex-vault"))} "$@"\n'
    )
    with tempfile.NamedTemporaryFile(
        mode="w", newline="", dir=bin_dir, delete=False
    ) as output:
        output.write(contents)
        temporary = Path(output.name)
    temporary.chmod(0o755)
    temporary.replace(vault)
    print(f"Installed Cowdex {version} at {installed}\nCommand: {launcher}")
    return installed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group()
    source.add_argument(
        "--archive", type=Path, help="Install a local .tar.gz or .zip package"
    )
    source.add_argument("--repo", help="Download from this GitHub OWNER/REPOSITORY")
    parser.add_argument("--release", default="latest")
    parser.add_argument(
        "--package-root",
        type=Path,
        default=Path(os.environ.get("CODEX_HOME", Path.home() / ".codex"))
        / "packages/cowdex",
    )
    parser.add_argument("--bin-dir", type=Path, default=Path.home() / ".local/bin")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="cowdex-download-") as tmp:
        package = Path(__file__).resolve().parent
        archive = args.archive
        if args.repo:
            archive = download_release(args.repo, args.release, Path(tmp))
        if archive:
            package = Path(tmp) / "package"
            package.mkdir()
            extract_archive(archive, package)
        install(
            package,
            args.package_root.expanduser().resolve(),
            args.bin_dir.expanduser().resolve(),
        )


if __name__ == "__main__":
    main()
