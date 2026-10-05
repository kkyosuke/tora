#!/usr/bin/env bash
set -euo pipefail

fail() { printf 'tora: %s\n' "$*" >&2; exit 1; }
version=${TORA_VERSION:-}
while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail '--version requires a value'
            version=$2; shift 2 ;;
        *) fail "unknown argument: $1" ;;
    esac
done
valid_version() { [[ "$1" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; }
if [ -n "$version" ]; then
    version="v${version#v}"
    valid_version "$version" || fail 'expected a stable version such as v0.1.0'
fi
for tool in curl tar awk mktemp; do
    command -v "$tool" >/dev/null || fail "$tool is required"
done
case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) platform=macos-arm64 ;;
    Darwin-x86_64) platform=macos-amd64 ;;
    Linux-x86_64) platform=linux-amd64 ;;
    *) fail 'supported platforms: macOS arm64/x86_64, Linux x86_64' ;;
esac
repo=https://github.com/KKyosuke/tora
fetch() { curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL --retry 3 --connect-timeout 15 --max-time 300 "$@"; }
if [ -z "$version" ]; then
    # Resolve once, then use the same immutable tag for all artifacts.
    latest=$(fetch -o /dev/null -w '%{url_effective}' "$repo/releases/latest")
    version=${latest##*/}
    valid_version "$version" || fail 'could not resolve a stable release; check GitHub Releases'
fi
root=${TORA_HOME:-${HOME:?HOME or TORA_HOME is required}/.tora}
bin="$root/bin"
mkdir -p "$bin"
# Fail closed on contention; never guess whether another updater is still alive.
lock="$root/update.lock"
mkdir "$lock" 2>/dev/null || fail "update lock exists: $lock (if no installer is running, remove the empty lock directory)"
stage=''
cleanup() {
    status=$?
    trap - EXIT
    [ -z "$stage" ] || rm -rf -- "$stage"
    rmdir "$lock" || true
    exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
stage=$(mktemp -d "$bin/.update.XXXXXXXX")
asset="tora-$platform.tar.gz"
base="$repo/releases/download/$version"
for file in "$asset" "$asset.sha256" "$asset.version"; do
    fetch "$base/$file" -o "$stage/$file"
done
checksum="$stage/$asset.sha256"
[ "$(wc -l < "$checksum" | tr -d ' ')" = 1 ] || fail 'invalid checksum file'
read -r expected name extra < "$checksum"
[[ "$expected" =~ ^[0-9a-fA-F]{64}$ ]] && [ "$name" = "$asset" ] && [ -z "${extra:-}" ] || fail 'invalid checksum record'
if command -v sha256sum >/dev/null; then
    actual=$(sha256sum "$stage/$asset" | awk '{print $1}')
elif command -v shasum >/dev/null; then
    actual=$(shasum -a 256 "$stage/$asset" | awk '{print $1}')
else
    fail 'sha256sum or shasum is required'
fi
[ "$actual" = "$(printf '%s' "$expected" | tr 'A-F' 'a-f')" ] || fail 'checksum mismatch'
[ "$(cat "$stage/$asset.version")" = "$version" ] || fail 'release version metadata mismatch'
[ "$(tar -tzf "$stage/$asset")" = tora ] || fail 'archive must contain exactly one top-level tora binary'
case "$(tar -tvzf "$stage/$asset")" in
    -*) ;;
    *) fail 'archive entry must be a regular file' ;;
esac
tar -xzf "$stage/$asset" -C "$stage" -- tora
chmod 755 "$stage/tora"
[ "$("$stage/tora" --version)" = "tora ${version#v}" ] || fail 'binary version mismatch'
[ ! -d "$bin/tora" ] || fail 'install target is a directory'
# Staging inside bin guarantees replacement by rename on the same filesystem.
mv -f -- "$stage/tora" "$bin/tora"
printf 'Installed tora %s at %s/tora\n' "$version" "$bin"
case ":$PATH:" in
    *":$bin:"*) ;;
    *) printf 'Add this directory to PATH in your shell configuration:\n  %s\n' "$bin" ;;
esac
