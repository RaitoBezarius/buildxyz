set -euxo pipefail
export BUILDXYZ_RELEASE_VARIANT="release"
export BUILDXYZ_BINARY="./target/$BUILDXYZ_RELEASE_VARIANT/buildxyz"
# Improve the performance of the evaluation
# because some packages believe it's fine to adopt nightly features
# in Python releases…
export RUSTC_BOOTSTRAP=1

export buildxyz_global_flags=()

if [[ -v AUTOMATIC ]]; then
  buildxyz_global_flags+=(--automatic)
fi

# Debugging infrastructure
export STRACE=""
export STRACE_EXTRA_FLAGS=""

if [[ -v ENABLE_STRACE ]]; then
  STRACE="strace -yy -f $STRACE_EXTRA_FLAGS"
fi

# Manual interaction
if [[ ! -v MANUAL ]]; then
  buildxyz_global_flags+=(--automatic)
fi

pypi_buildxyz() {
  package="$1"
  PREFIX_DIR=$(mktemp -d)
  echo "buildxyz $package in pip prefix $PREFIX_DIR"
  $STRACE $BUILDXYZ_BINARY "${buildxyz_global_flags[@]}" --record-to "examples/python/$package.toml" "pip install --use-feature=no-binary-enable-wheel-cache --prefix $PREFIX_DIR --no-binary :all: --no-cache-dir $package"
}

rubygems_buildxyz() {
  package="$1"
  PREFIX_DIR=$(mktemp -d)
  export GEM_HOME="$PREFIX_DIR"
  echo "buildxyz $package in gem prefix $PREFIX_DIR"
  $STRACE $BUILDXYZ_BINARY "${buildxyz_global_flags[@]}" --record-to "examples/ruby/$package.toml" "gem install --bindir $(mktemp -d) --install-dir $(mktemp -d) --no-user-install $package"
}
