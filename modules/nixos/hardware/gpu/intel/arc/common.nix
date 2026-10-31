{
  config,
  lib,
  options,
  pkgs,
  ...
}: let
  isArcA770 = builtins.elem config.canix-toolbelt.hardware.gpu.profile [
    "intel-arc-a770-i915"
    "intel-arc-a770-xe"
  ];
  isXe = config.canix-toolbelt.hardware.gpu.profile == "intel-arc-a770-xe";
in {
  imports = [../../profile.nix];

  config = lib.mkIf isArcA770 ({
      hardware.graphics = {
        enable = true;
        extraPackages = with pkgs; [
          intel-compute-runtime
          intel-media-driver
          level-zero
          libvdpau
          libvdpau-va-gl
          vkbasalt
          vpl-gpu-rt
          vulkan-loader
        ];
      };

      environment = {
        systemPackages = with pkgs; [
          clinfo
        ];

        variables = {
          LIBVA_DRIVER_NAME = "iHD";
          VDPAU_DRIVER = "va_gl";
        };
      };
    }
    // lib.optionalAttrs (options ? harbor-game.steam.extraArgs) {
      # Arc A770 + xe crashes steamwebhelper in Intel VA-API init, and CEF
      # still calls vaInitialize even when its GPU path is disabled. No-op
      # unless harbor-game's Steam module is imported.
      harbor-game.steam = lib.mkIf isXe {
        extraArgs = "-cef-disable-gpu";
        extraEnv.LIBVA_DRIVER_NAME = "dummy";
      };
    });
}
