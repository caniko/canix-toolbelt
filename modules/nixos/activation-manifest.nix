{
  config,
  lib,
  ...
}: let
  contracts = config.canix-toolbelt.activation.contracts;
  activation = import ../../lib/activation.nix {inherit lib;};
  manifest = activation.mkManifest contracts;
in {
  imports = [./activation-contracts.nix];

  config.environment.etc."canix/activation-contracts.json" = {
    mode = "0644";
    text = builtins.toJSON manifest;
  };
}
