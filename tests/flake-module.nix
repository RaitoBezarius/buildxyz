{ self, ... }: {
  perSystem = { lib, pkgs, ... }: {
    checks = lib.optionalAttrs pkgs.stdenv.isLinux {
      nixos-test = import ./nixos-test.nix {
        inherit self pkgs;
      };
    };
  };
}
