#!/usr/bin/env nix-shell
#!nix-shell --pure -i bash -p jq git nix rustc --keep BUILDXYZ_NIXPKGS
# shellcheck shell=sh

pypi_buildxyz() {
  package="$1"
  echo "buildxyz $package"
  ../target/debug/buildxyz --record-to "examples/python/$package.toml" "pip install $package --prefix /tmp --no-binary :all:"
}

pypi_buildxyz "$1"
