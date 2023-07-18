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
, fetchurl
, clippy
, path
, enableLint ? false
}:
let
  fuse = if stdenv.isDarwin then macfuse-stubs else fuse3;
  popcount-graph = builtins.fetchurl {
    url = "https://github.com/RaitoBezarius/buildxyz/releases/download/assets-0.1.0/popcount-graph.json";
    sha256 = "1xbhlcmb2laa9cp5qh9vsmmvzdifaqb7x7817ppjk1wx6gf2p02a";
  };
  nix-index-db = builtins.fetchurl {
    url = "https://github.com/RaitoBezarius/buildxyz/releases/download/assets-0.1.0/files";
    sha256 = "02igi3vkqg8hqwa9p03gyr6x2h99sz1gv2w4mzfw646qlckfh32p";
  };
in
rustPlatform.buildRustPackage
  {
    pname = "buildxyz";
    version = "0.0.1";
    src = runCommand "src" { } ''
      install -D ${./Cargo.toml} $out/Cargo.toml
      install -D ${./Cargo.lock} $out/Cargo.lock
      cp -r ${./src} $out/src
      ln -sf ${popcount-graph} $out/popcount-graph.json
      ln -sf ${nix-index-db} $out/nix-index-files
    '';
    # Use provided zstd rather than vendored one.
    ZSTD_SYS_USE_PKG_CONFIG = true;
    BUILDXYZ_NIXPKGS = path;
    BUILDXYZ_CORE_RESOLUTIONS = ./data;

    buildInputs = [ zstd fuse ];
    nativeBuildInputs = [ openssl cargo-flamegraph pkg-config ] ++ lib.optional enableLint clippy;

    shellHook = ''
      ln -s ${popcount-graph} popcount-graph.json
      ln -s ${nix-index-db} nix-index-files
    '';

    cargoLock = {
      lockFile = ./Cargo.lock;
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
