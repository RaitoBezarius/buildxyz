BUILDXYZ_RELEASE_VARIANT="release"
BUILDXYZ_BINARY="./target/$BUILDXYZ_RELEASE_VARIANT/buildxyz"

buildxyz_global_flags=()

# Debugging infrastructure
STRACE=""
STRACE_EXTRA_FLAGS=""

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
