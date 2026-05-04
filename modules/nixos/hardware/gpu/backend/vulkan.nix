{pkgs, ...}: {
  imports = [./common.nix];

  hardware.graphics.extraPackages = with pkgs; [
    libvdpau
    vkbasalt
    vulkan-loader
    vulkan-tools
    vulkan-validation-layers
  ];

  environment.systemPackages = with pkgs; [
    vulkan-tools
  ];
}
