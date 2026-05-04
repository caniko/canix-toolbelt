{pkgs, ...}: {
  hardware.graphics.enable = true;

  environment.systemPackages = with pkgs; [
    libva-utils
    mangohud
  ];
}
