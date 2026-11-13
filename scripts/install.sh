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
case "$root" in
    *:*|*$'\n'*|*$'\r'*) fail 'install path must not contain colon or newline characters' ;;
esac
mkdir -p "$root"
root=$(cd "$root" && pwd -P)
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
# The piped installer runs in Bash even for Zsh users; use the login shell.
configure_path() {
    local shell_name=${SHELL:-} file login index=0
    shell_name=${shell_name##*/}
    local files=()
    [ -n "${HOME:-}" ] || { printf 'HOME is unset; configure PATH manually.\n' >&2; return 1; }
    case "$shell_name" in
        zsh) files=("${ZDOTDIR:-$HOME}/.zshrc") ;;
        bash)
            files=("$HOME/.bashrc")
            login="$HOME/.bash_profile"
            for file in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
                if [ -e "$file" ] || [ -L "$file" ]; then login=$file; break; fi
            done
            files+=("$login") ;;
        *) printf 'Automatic PATH setup supports Bash/Zsh; current SHELL is %s.\n' "${SHELL:-unset}" >&2; return 1 ;;
    esac
    # Single-quote the path as literal shell data, including spaces and quotes.
    local quoted=${root//\'/\'\\\'\'}
    {
        printf '# >>> tora PATH >>>\n'
        printf "export TORA_HOME='%s'\n" "$quoted"
        cat <<'BLOCK'
case ":$PATH:" in
    *":$TORA_HOME/bin:"*) ;;
    *) export PATH="$TORA_HOME/bin:$PATH" ;;
esac
# <<< tora PATH <<<
BLOCK
    } > "$stage/path-block" || return 1
    # Prepare every file before writing, and refuse damaged/duplicate markers.
    for file in "${files[@]}"; do
        if [ -L "$file" ] && [ ! -f "$file" ]; then
            printf 'Cannot update broken/non-file symlink: %s\n' "$file" >&2; return 1
        fi
        if [ -e "$file" ] && { [ ! -f "$file" ] || [ ! -w "$file" ]; }; then
            printf 'Startup file is not writable: %s\n' "$file" >&2; return 1
        fi
        local input="$file"
        [ -e "$input" ] || input=/dev/null
        awk '
            FNR == NR { block = block $0 "\n"; next }
            $0 == "# >>> tora PATH >>>" { if (starts++ || inside || ends) exit 1; inside=1; printf "%s", block; next }
            $0 == "# <<< tora PATH <<<" { if (!inside || ends++) exit 1; inside=0; next }
            !inside { print }
            END {
                if (inside || starts != ends || starts > 1 || ends > 1) exit 1
                if (!starts) printf "\n%s", block
            }
        ' "$stage/path-block" "$input" > "$stage/startup-$index" || {
            printf 'Invalid tora PATH markers in %s; repair them before retrying.\n' "$file" >&2
            return 1
        }
        index=$((index + 1))
    done
    index=0
    for file in "${files[@]}"; do
        mkdir -p "${file%/*}" || return 1
        # Follow existing dotfile symlinks and retain their permissions.
        cat "$stage/startup-$index" > "$file" || return 1
        printf 'Configured PATH in %s\n' "$file"
        index=$((index + 1))
    done
    printf 'Open a new %s shell to use tora, or run in this shell:\n' "$shell_name"
    printf "  export TORA_HOME='%s'; export PATH=\"\$TORA_HOME/bin:\$PATH\"\n" "$quoted"
}
if ! configure_path; then
    printf 'tora: Installed successfully, but PATH setup needs attention. Add this directory to PATH: %s\n' "$bin" >&2
fi
