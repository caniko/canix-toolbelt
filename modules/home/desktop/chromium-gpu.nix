# Chromium GPU-acceleration wrapper (home-manager).
#
# Exposes `canix-toolbelt.chromiumGpu.wrap`, which wraps a Chromium-based
# package with Ozone selection and optional VA-API device routing. When
# canix-toolbelt.gpuMedia is enabled, --render-node-override and LIBVA_DRIVER_NAME
# select the device and driver. Chromium features are caller opt-in.
#
# `wrapper-manager` is injected by the flake (homeModules) so consumers need
# not thread `inputs.wrapper-manager` themselves.
{wrapper-manager}: {
  config,
  lib,
  pkgs,
  ...
}: let
  mkBackendWrapper = import ../../../lib/chromiumGpu.nix {inherit lib wrapper-manager;};
  effectiveGpuMedia = config.canix-toolbelt.gpuMedia;

  mkChromiumGpuWrapper = args: let
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
      ozonePlatform = wrapperArgs.ozonePlatform or "auto";
      extraFlags = wrapperArgs.extraFlags or [];
      extraFeatures = wrapperArgs.extraFeatures or [];
      extraEnv = wrapperArgs.extraEnv or {};
    };
in {
  options.canix-toolbelt.chromiumGpu.wrap = lib.mkOption {
    type = lib.types.raw;
    readOnly = true;
    description = "Wrap a Chromium-based package with canix-toolbelt GPU acceleration flags and environment.";
  };

  config.canix-toolbelt.chromiumGpu.wrap = mkChromiumGpuWrapper;
}
