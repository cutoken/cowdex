#!/bin/sh
# Install the complete Cowdex package using standard Unix tools.
set -eu

repo=cutoken/cowdex
release=latest
archive=
package_root=${COWDEX_PACKAGE_ROOT:-${CODEX_HOME:-$HOME/.codex}/packages/cowdex}
bin_dir=${COWDEX_INSTALL_DIR:-$HOME/.local/bin}
while [ "$#" -gt 0 ]; do
  case "$1" in
    --archive|--repo|--release|--package-root|--bin-dir)
      [ "$#" -ge 2 ] || { echo "Missing value for $1" >&2; exit 1; }
      case "$1" in
        --archive) archive=$2 ;; --repo) repo=$2 ;; --release) release=$2 ;;
        --package-root) package_root=$2 ;; --bin-dir) bin_dir=$2 ;;
      esac
      shift 2 ;;
    --help|-h)
      echo 'Usage: install.sh [--release VERSION] [--archive FILE] [--package-root DIR] [--bin-dir DIR]'
      exit 0 ;;
    *) echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
done
case "$(uname -s)" in
  Linux) platform=unknown-linux-musl ;; Darwin) platform=apple-darwin ;;
  *) echo 'Use install.ps1 on Windows.' >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;; aarch64|arm64) arch=aarch64 ;;
  *) echo 'Unsupported processor architecture.' >&2; exit 1 ;;
esac
target=$arch-$platform
case "$package_root" in /*) ;; *) package_root=$PWD/$package_root ;; esac
case "$bin_dir" in /*) ;; *) bin_dir=$PWD/$bin_dir ;; esac
mkdir -p "$package_root/releases"
scratch=$(mktemp -d "$package_root/.install-XXXXXX")
trap 'rm -rf "$scratch"' EXIT HUP INT TERM
if [ -z "$archive" ]; then
  printf '%s\n' "$repo" | grep -Eq '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' || { echo 'Invalid repository.' >&2; exit 1; }
  if [ "$release" = latest ]; then
    source_url=https://github.com/$repo/releases/latest/download
  else
    release=${release#cowdex-v}
    printf '%s\n' "$release" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-r[1-9][0-9]*)?$' || { echo 'Invalid release version.' >&2; exit 1; }
    source_url=https://github.com/$repo/releases/download/cowdex-v$release
  fi
  archive=$scratch/package.tar.gz
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --connect-timeout 15 --max-time 300 "$source_url/cowdex-$target.tar.gz" -o "$archive"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -T 300 -O "$archive" "$source_url/cowdex-$target.tar.gz"
  else
    echo 'Installing Cowdex requires curl or wget.' >&2; exit 1
  fi
fi
# Check entries before extraction; package archives contain only files and directories.
tar -tzf "$archive" > "$scratch/entries"
while IFS= read -r entry; do
  case "$entry" in
    /*|..|../*|*/../*|*/..|*\\*|*:*) echo "Unsafe archive path: $entry" >&2; exit 1 ;;
  esac
done < "$scratch/entries"
tar -tvzf "$archive" > "$scratch/types"
awk 'substr($0, 1, 1) != "-" && substr($0, 1, 1) != "d" { exit 1 }' "$scratch/types" || { echo 'Package archives cannot contain links.' >&2; exit 1; }
mkdir "$scratch/package"
tar -xzf "$archive" -C "$scratch/package"
metadata=$scratch/package/codex-package.json
metadata_value() {
  sed -n 's/.*"'"$1"'"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$metadata"
}
[ "$(metadata_value variant)" = cowdex ] || { echo 'Not a Cowdex package.' >&2; exit 1; }
package_target=$(metadata_value target)
case "$package_target" in
  "$target"|"$arch-unknown-linux-gnu")
    [ "$platform" = unknown-linux-musl ] || [ "$package_target" = "$target" ] || exit 1 ;;
  *) echo "Package must target $target." >&2; exit 1 ;;
esac
upstream_version=$(metadata_value version)
version=$(metadata_value releaseVersion)
version=${version:-$upstream_version}
printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-r[1-9][0-9]*)?$' || { echo 'Invalid package version.' >&2; exit 1; }
[ "${version%-r*}" = "$upstream_version" ] || { echo 'Release revision does not match package version.' >&2; exit 1; }
for helper in bin/cowdex bin/codex-code-mode-host bin/cowdex-vault codex-path/rg; do
  [ -x "$scratch/package/$helper" ] || { echo "Incomplete package: missing executable $helper" >&2; exit 1; }
done
if [ "$platform" = unknown-linux-musl ]; then
  [ -x "$scratch/package/codex-resources/bwrap" ] || { echo 'Incomplete package: missing bwrap.' >&2; exit 1; }
fi
"$scratch/package/bin/cowdex" --version
installed=$package_root/releases/$version-$package_target-$(date -u +%Y%m%d%H%M%S)-${scratch##*-}
mv "$scratch/package" "$installed"
ln -s "$installed" "$scratch/current"
# GNU mv uses -T; BSD mv uses -h to replace a symlink to a directory.
if mv -Tf "$scratch/current" "$package_root/current" 2>/dev/null; then :
elif mv -hf "$scratch/current" "$package_root/current" 2>/dev/null; then :
else rm -f "$package_root/current"; mv -f "$scratch/current" "$package_root/current"
fi
mkdir -p "$bin_dir"
shell_quote() {
  printf "'"
  printf '%s' "$1" | sed "s/'/'\\\\''/g"
  printf "'"
}
for program in cowdex cowdex-vault; do
  launcher=$(mktemp "$bin_dir/.$program.XXXXXX")
  if [ "$program" = cowdex ]; then
    printf '#!/bin/sh\nexec %s -c features.multi_agent_v2.preserve_fork_prefix=true "$@"\n' "$(shell_quote "$installed/bin/$program")" > "$launcher"
  else
    printf '#!/bin/sh\nexec %s "$@"\n' "$(shell_quote "$installed/bin/$program")" > "$launcher"
  fi
  chmod 755 "$launcher"
  mv -f "$launcher" "$bin_dir/$program"
done
printf 'Installed Cowdex %s at %s\nCommand: %s/cowdex\n' "$version" "$installed" "$bin_dir"
