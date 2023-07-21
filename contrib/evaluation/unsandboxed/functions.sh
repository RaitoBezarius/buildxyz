export BUILDXYZ_RELEASE_VARIANT="release"
export BUILDXYZ_BINARY="./target/$BUILDXYZ_RELEASE_VARIANT/buildxyz"

export buildxyz_global_flags=()

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
  echo $STRACE $BUILDXYZ_BINARY --record-to "examples/python/$package.toml" "pip install --use-feature=no-binary-enable-wheel-cache --prefix $PREFIX_DIR --no-binary :all: --no-cache-dir $package"
  $STRACE $BUILDXYZ_BINARY "${buildxyz_global_flags[@]}" --record-to "examples/python/$package.toml" "pip install --use-feature=no-binary-enable-wheel-cache --prefix $PREFIX_DIR --no-binary :all: --no-cache-dir $package"
}
