#!/usr/bin/env python3
"""Export current source to an empty directory, without private Git history."""

import argparse
from pathlib import Path
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    destination = args.destination.resolve()
    if destination == root or root in destination.parents:
        parser.error("Destination must be outside the development checkout")
    if destination.exists() and any(destination.iterdir()):
        parser.error("Destination must be empty; existing files are never overwritten")
    paths = (
        subprocess.check_output(
            ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            cwd=root,
        )
        .decode()
        .split("\0")
    )
    selected = []
    for name in sorted(set(paths) - {""}):
        path = Path(name)
        if (
            path.name == "HANDOFF.md"
            or name == "build_for_mac.md"
            or name == ".github/dependabot.yaml"
            or name.endswith(".snap.new")
        ):
            continue
        if (
            name.startswith(".github/workflows/")
            and name != ".github/workflows/cowdex-release.yml"
        ):
            continue
        if (root / path).is_file() or (root / path).is_symlink():
            selected.append(path)
    destination.mkdir(parents=True, exist_ok=True)
    for path in selected:
        source = root / path
        target = destination / path
        target.parent.mkdir(parents=True, exist_ok=True)
        if source.is_symlink():
            target.symlink_to(source.readlink())
        else:
            shutil.copy2(source, target)
    print(f"Exported {len(selected)} source files to {destination}")
    print(
        "No Git history or remotes were copied. Review the snapshot before publishing."
    )


if __name__ == "__main__":
    main()
