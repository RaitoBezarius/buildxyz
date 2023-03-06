{ inputs, ... }: {
  imports = [
    inputs.treefmt-nix.flakeModule
  ];

  perSystem =
    { pkgs
    , lib
    , ...
    }: {
      treefmt = {
        # Used to find the project root
        projectRootFile = "flake.lock";

        programs.rustfmt.enable = true;

        settings.formatter = {
          nix = {
            command = pkgs.runtimeShell;
            options = [
              "-eucx"
              ''
                ${lib.getExe pkgs.deadnix} --edit "$@"
                ${lib.getExe pkgs.nixpkgs-fmt} "$@"
              ''
              "--"
            ];
            includes = [ "*.nix" ];
          };

          shell = {
            command = pkgs.runtimeShell;
            options = [
              "-eucx"
              ''
                ${pkgs.lib.getExe pkgs.shellcheck} --external-sources --source-path=SCRIPTDIR "$@"
                ${pkgs.lib.getExe pkgs.shfmt} -i 2 -s -w "$@"
              ''
              "--"
            ];
            includes = [ "*.sh" ];
          };
        };
      };
    };
}
