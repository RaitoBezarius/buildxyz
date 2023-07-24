#!/usr/bin/env nix-shell
#!nix-shell --pure -i bash -p jq git nix bubblewrap which parallel tmux --keep BUILDXYZ_NIXPKGS --keep RUST_BACKTRACE --keep ENABLE_STRACE --keep NIX_DEBUG --keep MANUAL
# shellcheck shell=sh

SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
. "$SCRIPT_DIR/functions.sh"

JOB="${1:-ruby-job}"
export -f rubygems_buildxyz

while IFS=',' read -ra TOP_RUBY_PACKAGES; do
  RUBY_PACKAGES+=("${TOP_RUBY_PACKAGES[0]}")
done < top-rubygems.csv


mkdir -p "$TMPDIR/job-logs/$JOB"
parallel --output-as-files --results "$TMPDIR/job-logs/$JOB" --resume-failed --joblog $JOB --progress --bar --delay 2.5 --jobs 25% --tmuxpane rubygems_buildxyz ::: "${RUBY_PACKAGES[@]}"
