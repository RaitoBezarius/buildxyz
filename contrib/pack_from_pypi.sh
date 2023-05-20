#!/usr/bin/env nix-shell
#!nix-shell --pure -i bash -p jq git nix bubblewrap which --keep BUILDXYZ_NIXPKGS --keep RUST_BACKTRACE
# shellcheck shell=sh

pypi_buildxyz() {
  package="$1"
  echo "buildxyz $package"
  export TMPDIR="/buildxyz"
  bwrap \
  --bind /nix /nix \
  --dev-bind /dev /dev \
  --ro-bind $(which git) $(which git) \
  --ro-bind $(pwd)/target $(pwd)/target \
  --bind $(pwd)/examples $(pwd)/examples \
  --tmpfs /buildxyz \
  --proc /proc \
  --unshare-pid \
  --cap-add CAP_SYS_ADMIN \
  --new-session \
    ./target/debug/buildxyz --record-to "examples/python/$package.toml" "pip install $package --prefix /tmp --no-binary :all:"
}

pypi_buildxyz "$1"
