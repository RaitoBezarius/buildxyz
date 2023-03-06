# The first argument to this function is the test module itself         Jörg Thalheim, Previous month ◉ add lightning-knd test
test:
# These arguments are provided by `flake.nix` on import, see checkArgs
{ pkgs, self }:
let
  inherit (pkgs) lib;
  nixos-lib = import (pkgs.path + "/nixos/lib") { };
in
(nixos-lib.runTest {
  hostPkgs = pkgs;
  # optional to speed up to evaluation by skipping evaluating documentation
  defaults.documentation.enable = lib.mkDefault false;
  # This makes `self` available in the nixos configuration of our virtual machines.
  # This is useful for referencing modules or packages from your own flake as well as importing
  # from other flakes.
  node.specialArgs = { inherit self; };
  _module.args = { inherit self; };
  imports = [ test ];
}).config.result
