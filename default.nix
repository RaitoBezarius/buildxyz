{ fuse3
, pkg-config
, rustPlatform
, self ? ./.
, lib
}:
rustPlatform.buildRustPackage {
  name = "buildxzy";
  src = self;
  buildInputs = [ fuse3 ];
  nativeBuildInputs = [ pkg-config ];
  cargoLock = {
    lockFile = ./Cargo.lock;
  };
  meta = with lib; {
    description = "Provides build shell that can automatically figure out dependencies";
    homepage = "https://github.com/RaitoBezarius/buildxyz";
    license = licenses.mit;
  };
}
