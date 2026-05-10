# Construct a `pkgs` instance with `allowUnfree` and GPU-vendor toggles
# (`cudaSupport`, `rocmSupport`, `nvidia.acceptLicense`) derived from a
# normalized GPU record (see `lib/gpu.nix`).
#
# Example:
#
#   let
#     gpu = canix-toolbelt.lib.gpu.normalize { dgpu = "nvidia"; };
#     pkgs = canix-toolbelt.lib.mkPkgs {
#       input = inputs.nixpkgs-cuda;
#       inherit gpu;
#       system = "x86_64-linux";
#     };
#   in pkgs.cudaPackages.cuda_nvcc
{
  input,
  system,
  gpu,
  overlays ? [],
  extraConfig ? {},
}:
import input {
  inherit system overlays;
  config =
    {
      allowUnfree = true;
      nvidia.acceptLicense = gpu.hasNvidia or false;
      rocmSupport = gpu.hasAmd or false;
      cudaSupport = gpu.hasNvidia or false;
    }
    // extraConfig;
}
