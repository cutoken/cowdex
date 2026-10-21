"""Exercise package contents and platform installers without compiling Rust."""

import json
import os
from pathlib import Path
import subprocess
import sysconfig
import tarfile
import tempfile
import unittest
import zipfile

from codex_package.archive import write_archive
from codex_package.layout import build_package_dir, validate_package_dir
from codex_package.targets import (
    PACKAGE_VARIANTS,
    PackageInputs,
    TARGET_SPECS,
    default_target,
)


class CowdexReleaseTest(unittest.TestCase):
    def make_package(self, root, *, binary=None, spec=None):
        spec = spec or TARGET_SPECS[default_target()]
        if binary is None:
            binary = root / "fixture"
            binary.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
            binary.chmod(0o755)
        package = root / "package"
        package.mkdir()
        inputs = PackageInputs(
            entrypoint_bin=binary,
            code_mode_host_bin=binary,
            rg_bin=binary,
            zsh_bin=None,
            bwrap_bin=binary if spec.is_linux else None,
            codex_command_runner_bin=binary if spec.is_windows else None,
            codex_windows_sandbox_setup_bin=binary if spec.is_windows else None,
            vault_bin=binary,
        )
        build_package_dir(
            package,
            "1.2.3",
            PACKAGE_VARIANTS["cowdex"],
            spec,
            inputs,
            release_version="1.2.3-r2",
        )
        validate_package_dir(
            package, PACKAGE_VARIANTS["cowdex"], spec, include_zsh=False
        )
        return package

    def test_archive_roundtrip_bundles_helpers_vault_and_platform_installers(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = self.make_package(root)
            original = {
                p.relative_to(package).as_posix(): p.read_bytes()
                for p in package.rglob("*")
                if p.is_file()
            }
            for extension in ("tar.gz", "zip"):
                archive = root / f"package.{extension}"
                write_archive(package, archive, force=False)
                if extension == "zip":
                    with zipfile.ZipFile(archive) as source:
                        actual = {
                            entry.filename: source.read(entry)
                            for entry in source.infolist()
                            if not entry.is_dir()
                        }
                else:
                    with tarfile.open(archive) as source:
                        actual = {
                            entry.name: source.extractfile(entry).read()
                            for entry in source
                            if entry.isfile()
                        }
                self.assertEqual(actual, original)

    def run_installer(self, root, archive):
        return subprocess.run(
            [
                "sh",
                "-s",
                "--",
                "--archive",
                str(archive),
                "--package-root",
                str(root / "installed"),
                "--bin-dir",
                str(root / "bin"),
            ],
            input=Path(__file__).with_name("install.sh").read_text(),
            text=True,
            capture_output=True,
        )

    @unittest.skipUnless(os.name == "nt", "Windows installer regression check")
    def test_windows_installer_uses_native_architecture_inside_x64_build_shell(self):
        with tempfile.TemporaryDirectory(prefix="cowdex windows install ") as tmp:
            root = Path(tmp)
            arch = "arm64" if sysconfig.get_platform() == "win-arm64" else "amd64"
            target = "aarch64" if arch == "arm64" else "x86_64"
            source = root / "fixture.go"
            source.write_text(
                'package main\nimport "fmt"\nfunc main() { fmt.Println("fixture") }\n'
            )
            binary = root / "fixture.exe"
            subprocess.run(
                ["go", "build", "-o", str(binary), str(source)],
                env={
                    **os.environ,
                    "GOOS": "windows",
                    "GOARCH": arch,
                    "CGO_ENABLED": "0",
                },
                check=True,
            )
            package = self.make_package(
                root, binary=binary, spec=TARGET_SPECS[f"{target}-pc-windows-msvc"]
            )
            archive = root / "package.zip"
            write_archive(package, archive, force=False)
            result = subprocess.run(
                [
                    "powershell.exe",
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    str(Path(__file__).with_name("install.ps1")),
                    "-Archive",
                    str(archive),
                    "-PackageRoot",
                    str(root / "installed"),
                    "-BinDir",
                    str(root / "bin"),
                ],
                env={
                    **os.environ,
                    "PROCESSOR_ARCHITECTURE": "AMD64",
                    "PROCESSOR_ARCHITEW6432": "AMD64",
                },
                text=True,
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"1.2.3-r2-{target}-pc-windows-msvc", result.stdout)
            self.assertEqual(
                subprocess.check_output(
                    [str(root / "bin/cowdex.cmd"), "--version"], text=True
                ).strip(),
                "fixture",
            )

    @unittest.skipIf(
        os.name == "nt", "Windows installation is exercised by the release workflow"
    )
    def test_shell_install_preserves_old_releases_regular_codex_and_arguments(self):
        with tempfile.TemporaryDirectory(prefix="cowdex's install ") as tmp:
            root = Path(tmp)
            package = self.make_package(root)
            archive = root / "package.tar.gz"
            write_archive(package, archive, force=False)
            (root / "bin").mkdir()
            regular = root / "bin/codex-code-mode-host"
            regular.write_text("regular Codex host")
            for _ in range(2):
                result = self.run_installer(root, archive)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("Installed Cowdex 1.2.3-r2", result.stdout)
            releases = [
                path.resolve() for path in (root / "installed/releases").iterdir()
            ]
            self.assertEqual(len(releases), 2)
            current = (root / "installed/current").resolve()
            self.assertIn(current, releases)
            metadata = json.loads((current / "codex-package.json").read_text())
            self.assertEqual(
                (metadata["version"], metadata["releaseVersion"]), ("1.2.3", "1.2.3-r2")
            )
            self.assertEqual(regular.read_text(), "regular Codex host")
            self.assertEqual(
                subprocess.check_output(
                    [root / "bin/cowdex", "hello world"], text=True
                ).splitlines(),
                [
                    "-c",
                    "features.multi_agent_v2.preserve_fork_prefix=true",
                    "hello world",
                ],
            )
            self.assertEqual(
                subprocess.check_output(
                    [root / "bin/cowdex-vault", "--version"], text=True
                ),
                "--version\n",
            )

    @unittest.skipIf(
        os.name == "nt", "Windows installation is exercised by the release workflow"
    )
    def test_incomplete_or_wrong_platform_package_cannot_replace_launcher(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = self.make_package(root)
            metadata_path = package / "codex-package.json"
            metadata = json.loads(metadata_path.read_text())
            expected = metadata["target"]
            metadata["target"] = "not-this-platform"
            metadata_path.write_text(json.dumps(metadata))
            archive = root / "package.tar.gz"
            write_archive(package, archive, force=False)
            self.assertNotEqual(self.run_installer(root, archive).returncode, 0)
            metadata["target"] = expected
            metadata_path.write_text(json.dumps(metadata))
            (package / "bin/codex-code-mode-host").unlink()
            write_archive(package, archive, force=True)
            self.assertNotEqual(self.run_installer(root, archive).returncode, 0)
            self.assertFalse((root / "bin").exists())

    @unittest.skipIf(
        os.name == "nt", "Windows installation is exercised by the release workflow"
    )
    def test_rejects_archive_paths_outside_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / "bad.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                output.addfile(tarfile.TarInfo("../escaped"))
            result = self.run_installer(root, archive)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Unsafe archive path", result.stderr)
            self.assertFalse((root / "escaped").exists())


if __name__ == "__main__":
    unittest.main()
