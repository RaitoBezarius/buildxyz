#!/usr/bin/env nix-shell
#!nix-shell --pure -i bash -p jq git nix bubblewrap which parallel tmux --keep BUILDXYZ_NIXPKGS --keep RUST_BACKTRACE --keep ENABLE_STRACE --keep NIX_DEBUG --keep MANUAL
# shellcheck shell=sh

SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
. "$SCRIPT_DIR/functions.sh"

JOB="${1:-pypi-job}"
export -f pypi_buildxyz

readarray -t PYPI_PACKAGES < <(jq -rc '.rows | .[] | .project' top-pypi.json)
parallel --joblog $JOB --progress --bar --delay 2.5 --jobs 50% --tmux pypi_buildxyz ::: "${PYPI_PACKAGES[@]}"
