{pkgs, ...}: {
  hardware.graphics.extraPackages = with pkgs; [
    mesa
    libvdpau-va-gl
  ];
}
