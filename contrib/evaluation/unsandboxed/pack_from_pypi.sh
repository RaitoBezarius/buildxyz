#!/usr/bin/env nix-shell
#!nix-shell --pure -i bash -p jq git nix which cacert strace --keep BUILDXYZ_NIXPKGS --keep RUST_BACKTRACE --keep MANUAL --keep ENABLE_STRACE --keep NIX_DEBUG
# shellcheck shell=sh

SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
. "$SCRIPT_DIR/functions.sh"

pypi_buildxyz "$1"
