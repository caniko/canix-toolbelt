{...}: {
  # This module contains only the shared contract option tree and assertions;
  # it has no NixOS-only configuration.
  imports = [../nixos/activation-contracts.nix];
}
