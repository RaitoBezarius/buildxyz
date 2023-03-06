{
  description = "Development environment for this project";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";

    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
  };

  outputs = inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } ({ ... }: {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        # TODO: fix eval...
        #"riscv64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      imports = [
        ./treefmt/flake-module.nix
      ];

      perSystem = { self', pkgs, ... }: {
        packages.buildxyz = pkgs.callPackage ./default.nix { };
        packages.default = self'.packages.buildxyz;
        checks.clippy = self'.packages.buildxyz.override {
          enableLint = true;
        };
      };
    });
}
