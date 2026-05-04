{lib, ...}: {
  imports = [../profile.nix];

  hardware.intel-gpu-tools.enable = lib.mkDefault true;
}
