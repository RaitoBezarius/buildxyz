#!/usr/bin/env nix-shell
#!nix-shell --pure -i bash -p jq git nix bubblewrap which cacert strace --keep BUILDXYZ_NIXPKGS --keep RUST_BACKTRACE
# shellcheck shell=sh

buildxyz_global_flags=()

if [[ -v AUTOMATIC ]]; then
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
    ./target/debug/buildxyz "${buildxyz_global_flags[@]}" --record-to "examples/python/$package.toml" "pip install $package --prefix /tmp --no-binary :all:"
}

pypi_buildxyz "$1"
