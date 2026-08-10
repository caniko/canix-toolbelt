# Chromium GPU-acceleration wrapper (home-manager).
#
# Exposes `canix-toolbelt.chromiumGpu.wrap`, which wraps a Chromium-based
# package with VA-API video-accel feature flags + the matching env. Decode
# follows canix-toolbelt.gpuMedia: when the media route is enabled, decode is
# pinned to it via --render-node-override + LIBVA_DRIVER_NAME; otherwise pure
# rendering wrappers are produced (Wayland/ANGLE only).
#
# `wrapper-manager` is injected by the flake (homeModules) so consumers need
# not thread `inputs.wrapper-manager` themselves.
{wrapper-manager}: {
  config,
  lib,
  pkgs,
  gpuMedia ? null,
  ...
}: let
  mkBackendWrapper = import ../../../lib/chromiumGpu.nix {inherit lib wrapper-manager;};
  effectiveGpuMedia =
    if gpuMedia != null
    then gpuMedia
    else config.canix-toolbelt.gpuMedia or {};

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
