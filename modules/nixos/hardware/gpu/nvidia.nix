{pkgs, ...}: {
  hardware.graphics.extraPackages = [pkgs.nvidia-vaapi-driver];
}
