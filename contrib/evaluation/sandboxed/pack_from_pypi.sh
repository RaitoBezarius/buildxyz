#!/usr/bin/env nix-shell
#!nix-shell --pure -i bash -p gcc jq git nix bubblewrap which cacert strace --keep BUILDXYZ_NIXPKGS --keep RUST_BACKTRACE --keep MANUAL --keep ENABLE_STRACE --keep NIX_DEBUG
# shellcheck shell=sh

buildxyz_global_flags=()
STRACE=""

if [[ -v ENABLE_STRACE ]]; then
  STRACE="strace -yy -e file -f"
fi

if [[ ! -v MANUAL ]]; then
  buildxyz_global_flags+=(--automatic)
fi

pypi_buildxyz() {
  package="$1"
  echo "buildxyz $package"
  export TMPDIR="/buildxyz"
  bwrap \
  --ro-bind /etc/resolv.conf /etc/resolv.conf \
  --share-net \
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
    $STRACE ./target/debug/buildxyz "${buildxyz_global_flags[@]}" --record-to "examples/python/$package.toml" "pip install --verbose $package --prefix /tmp --no-binary :all:"
}

pypi_buildxyz "$1"
