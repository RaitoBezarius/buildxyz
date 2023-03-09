{ fuse3
, macfuse-stubs
, stdenv
, pkg-config
, openssl
, zstd
, cargo-flamegraph
, rustPlatform
, lib
, runCommand
, clippy
, enableLint ? false
}:
let
  fuse = if stdenv.isDarwin then macfuse-stubs else fuse3;
in
rustPlatform.buildRustPackage
  {
    pname = "buildxyz";
    version = "0.0.1";
    src = runCommand "src" { } ''
      install -D ${./Cargo.toml} $out/Cargo.toml
      install -D ${./Cargo.lock} $out/Cargo.lock
      cp -r ${./src} $out/src
    '';
    buildInputs = [ fuse openssl zstd ];
    nativeBuildInputs = [ cargo-flamegraph pkg-config ] ++ lib.optional enableLint clippy;
    cargoLock = {
      lockFile = ./Cargo.lock;
     # outputHashes = {
     #   "nix-index-0.1.5" = "sha256-/btQP7I4zpIA0MWEQJVYnR1XhyudPnYD5Qx4vrW+Uq8=";
     # };
    };
    meta = with lib; {
      description = "Provides build shell that can automatically figure out dependencies";
      homepage = "https://github.com/RaitoBezarius/buildxyz";
      license = licenses.mit;
    };
  } // lib.optionalAttrs enableLint {
  buildPhase = ''
    cargo clippy --all-targets --all-features -- -D warnings
    if grep -R 'dbg!' ./src; then
      echo "use of dbg macro found in code!"
      false
    fi
  '';

  installPhase = ''
    touch $out
  '';
}
