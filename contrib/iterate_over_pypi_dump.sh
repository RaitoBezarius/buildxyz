#!/usr/bin/env nix-shell
#!nix-shell --pure -i bash -p jq git nix bubblewrap which cacert parallel tmux --keep BUILDXYZ_NIXPKGS --keep RUST_BACKTRACE
# shellcheck shell=sh

pypi_buildxyz() {
  package="$1"
  echo "buildxyz $package"
  # This is needed for the new tmpfs
  export TMPDIR="/buildxyz"
 # CAP_SYS_ADMIN is for the fusermount
 # /dev bind is for /dev/fuse
 # --share-net is necessary for network interactions.
 # share also the DNS resolver.
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
    ./target/debug/buildxyz --automatic --record-to "examples/python/$package.toml" "pip install $package --prefix /tmp --no-binary :all:"
}

export -f pypi_buildxyz

readarray -t PACKAGES < <(jq -rc '.rows | .[] | .project' top-pypi.json)
parallel --progress --bar --delay 2.5 --jobs 50% --tmux pypi_buildxyz ::: "${PACKAGES[@]}"
