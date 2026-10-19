"""Exercise package contents and installation without compiling Rust."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

from codex_package.archive import write_archive
from codex_package.layout import build_package_dir, validate_package_dir
from codex_package.targets import PACKAGE_VARIANTS, PackageInputs, TARGET_SPECS
from install_cowdex import extract_archive, host_target, install


class CowdexReleaseTest(unittest.TestCase):
    def make_package(self, root):
        target = host_target()
        spec = TARGET_SPECS[target]
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
        )
        build_package_dir(package, "1.2.3", PACKAGE_VARIANTS["cowdex"], spec, inputs)
        validate_package_dir(
            package, PACKAGE_VARIANTS["cowdex"], spec, include_zsh=False
        )
        return package

    def test_archive_roundtrip_bundles_helpers_vault_and_installer(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = self.make_package(root)
            for extension in ("tar.gz", "zip"):
                archive = root / f"package.{extension}"
                write_archive(package, archive, force=False)
                extracted = root / extension
                extracted.mkdir()
                extract_archive(archive, extracted)
                original = {
                    p.relative_to(package): p.read_bytes()
                    for p in package.rglob("*")
                    if p.is_file()
                }
                actual = {
                    p.relative_to(extracted): p.read_bytes()
                    for p in extracted.rglob("*")
                    if p.is_file()
                }
                self.assertEqual(actual, original)

    @unittest.skipIf(os.name == "nt", "The curl quickstart targets Unix shells")
    def test_installer_runs_from_stdin(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = self.make_package(root)
            archive = root / "cowdex.tar.gz"
            write_archive(package, archive, force=False)
            result = subprocess.run(
                [
                    sys.executable,
                    "-",
                    "--archive",
                    str(archive),
                    "--package-root",
                    str(root / "installed"),
                    "--bin-dir",
                    str(root / "bin"),
                ],
                input=Path(__file__).with_name("install_cowdex.py").read_text(),
                text=True,
                capture_output=True,
                check=True,
            )
            self.assertIn("Installed Cowdex 1.2.3", result.stdout)
            self.assertTrue((root / "bin/cowdex").is_file())

    def test_install_keeps_previous_releases_and_codex_helpers_untouched(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = self.make_package(root)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            regular_helper = bin_dir / "codex-code-mode-host"
            regular_helper.write_text("regular Codex host")
            with patch("install_cowdex.subprocess.run"):
                first = install(package, root / "releases", bin_dir)
                second = install(package, root / "releases", bin_dir)
            self.assertTrue(first.is_dir())
            self.assertTrue(second.is_dir())
            self.assertNotEqual(first, second)
            self.assertEqual(regular_helper.read_text(), "regular Codex host")
            if os.name != "nt":
                output = subprocess.check_output(
                    [bin_dir / "cowdex", "hello world"], text=True
                )
                self.assertEqual(
                    output.splitlines(),
                    [
                        "-c",
                        "features.multi_agent_v2.preserve_fork_prefix=true",
                        "hello world",
                    ],
                )

    def test_incomplete_or_wrong_platform_package_cannot_replace_launcher(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = self.make_package(root)
            metadata_path = package / "codex-package.json"
            metadata = json.loads(metadata_path.read_text())
            metadata["target"] = "not-this-platform"
            metadata_path.write_text(json.dumps(metadata))
            with self.assertRaisesRegex(ValueError, "release for"):
                install(package, root / "releases", root / "bin")
            metadata["target"] = host_target()
            metadata_path.write_text(json.dumps(metadata))
            suffix = ".exe" if os.name == "nt" else ""
            (package / f"bin/codex-code-mode-host{suffix}").unlink()
            with self.assertRaisesRegex(ValueError, "missing bin/codex-code-mode-host"):
                install(package, root / "releases", root / "bin")
            self.assertFalse((root / "bin").exists())

    def test_rejects_archive_paths_outside_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / "bad.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                output.addfile(tarfile.TarInfo("../escaped"))
            with self.assertRaisesRegex(ValueError, "Unsafe archive path"):
                extract_archive(archive, root / "package")
            self.assertFalse((root / "escaped").exists())


if __name__ == "__main__":
    unittest.main()
