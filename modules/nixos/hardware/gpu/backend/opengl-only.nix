{pkgs, ...}: {
  imports = [./common.nix];

  hardware.graphics.extraPackages = with pkgs; [
    libvdpau-va-gl
  ];
}
