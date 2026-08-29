# Firefox/Floorp GPU media wrapper (home-manager).
#
# Exposes `canix-toolbelt.firefoxGpu.wrap`, which wraps a Firefox-family
# package with MOZ_DRM_DEVICE + LIBVA_DRIVER_NAME pointing hardware video
# decode at the canix-toolbelt.gpuMedia route. Rendering is left to Wayland.
#
# `wrapper-manager` is injected by the flake (homeModules).
{wrapper-manager}: {
  config,
  lib,
  pkgs,
  gpuMedia ? null,
  ...
}: let
  mkBackendWrapper = import ../../../lib/firefoxGpu.nix {inherit lib wrapper-manager;};
  effectiveGpuMedia =
    if gpuMedia != null
    then gpuMedia
    else config.canix-toolbelt.gpuMedia or {};

  mkFirefoxGpuWrapper = args: let
    wrapperArgs =
      if lib.isDerivation args
      then {basePackage = args;}
      else args;
    inherit (wrapperArgs) basePackage;
    wrapperName = wrapperArgs.wrapperName or (lib.getName basePackage);
    skipPrograms = wrapperArgs.skipPrograms or [];
  in
    mkBackendWrapper {
      inherit pkgs basePackage wrapperName skipPrograms;
      gpuMedia = effectiveGpuMedia;
      extraEnv = wrapperArgs.extraEnv or {};
    };
in {
  options.canix-toolbelt.firefoxGpu.wrap = lib.mkOption {
    type = lib.types.raw;
    readOnly = true;
    description = "Wrap a Firefox-family package with MOZ_DRM_DEVICE + LIBVA_DRIVER_NAME for the media GPU route.";
  };

  config.canix-toolbelt.firefoxGpu.wrap = mkFirefoxGpuWrapper;
}
