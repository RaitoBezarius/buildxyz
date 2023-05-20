#!/usr/bin/env nix-shell
#!nix-shell --pure -i bash -p jq git nix --keep BUILDXYZ_NIXPKGS
# shellcheck shell=sh

pypi_buildxyz() {
  package="$1"
  echo "buildxyz $package"
  ../target/debug/buildxyz --automatic --record-to "examples/python/$package.toml" "pip install $package --prefix /tmp --no-binary :all:"
}

jq -rc '.rows | .[] | .project' top-pypi.json | while read -r package; do
  pypi_buildxyz "$package"
done
