#!/bin/bash
# Cargo runner: isolate runtime state without changing release binaries.
set -euo pipefail
[ "$#" -gt 0 ] || { printf 'usage: run-dev.sh EXECUTABLE [ARGS...]\n' >&2; exit 2; }
script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
dev_root="${script_dir%/scripts}/.cache/dev"
mkdir -p "$dev_root/home/.aws" "$dev_root/tora" "$dev_root/tmp" \
    "$dev_root/config" "$dev_root/cache" "$dev_root/data" "$dev_root/state"

# The installer exports TORA_HOME in the user's startup files. Always replace it
# here, rather than using it as a fallback or inheriting a production directory.
export HOME="$dev_root/home"
export TORA_HOME="$dev_root/tora"
export ZDOTDIR="$HOME"
export TMPDIR="$dev_root/tmp"
export XDG_CONFIG_HOME="$dev_root/config"
export XDG_CACHE_HOME="$dev_root/cache"
export XDG_DATA_HOME="$dev_root/data"
export XDG_STATE_HOME="$dev_root/state"
unset BASH_ENV ENV
# Isolate AWS config, cached SSO tokens, profile/region and credential providers.
# Explicitly configure and log in inside this development environment instead.
for name in ${!AWS_@}; do unset "$name"; done
export AWS_CONFIG_FILE="$HOME/.aws/config"
export AWS_SHARED_CREDENTIALS_FILE="$HOME/.aws/credentials"
export AWS_EC2_METADATA_DISABLED=true

executable=$1
shift
case "$executable" in /*) ;; *) executable="$PWD/$executable" ;; esac
# An AWS exec/SSO child invoking `tora` must use the same development binary.
if [ "${executable##*/}" = tora ]; then
    export PATH="${executable%/*}:$PATH"
fi
exec "$executable" "$@"
