{pkgs, ...}: {
  # Intel HD Graphics 615 (Kaby Lake); no Vulkan support.
  imports = [
    ../backend/opengl-only.nix
    ./common.nix
    ./media.nix
  ];

  hardware.graphics.extraPackages = with pkgs; [
    intel-ocl
    vulkan-loader
    vulkan-tools
    vulkan-validation-layers
  ];

  environment.variables = {
    ANV_DEBUG = "video-decode,video-encode";
  };
}
