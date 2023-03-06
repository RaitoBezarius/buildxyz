(import ./lib.nix) ({ pkgs, ... }: {
  name = "from-nixos";
  nodes = {
    # self here is set by using specialArgs in `lib.nix`
    node1 = { self, ... }: {
      environment.systemPackages = [
        self.packages.${pkgs.targetPlatform.system}.buildxyz
      ];
    };
  };

  # This test is still wip
  testScript = ''
    start_all()

    node1.succeed("buildxyz")
  '';
})
