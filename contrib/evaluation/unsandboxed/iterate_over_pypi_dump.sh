#!/usr/bin/env nix-shell
#!nix-shell --pure -i bash -p jq git nix bubblewrap which parallel tmux --keep BUILDXYZ_NIXPKGS --keep RUST_BACKTRACE --keep ENABLE_STRACE --keep NIX_DEBUG --keep MANUAL
# shellcheck shell=sh

SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
. "$SCRIPT_DIR/functions.sh"

JOB="${1:-pypi-job}"
export -f pypi_buildxyz

readarray -t PYPI_PACKAGES < <(jq -rc '.rows | .[] | .project' top-pypi.json)
mkdir -p "$TMPDIR/job-logs/$JOB"
parallel --output-as-files --results "$TMPDIR/job-logs/$JOB" --resume-failed --joblog $JOB --progress --bar --delay 2.5 --jobs 25% --tmuxpane pypi_buildxyz ::: "${PYPI_PACKAGES[@]}"
