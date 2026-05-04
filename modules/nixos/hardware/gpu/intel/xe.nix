{
  config,
  lib,
  options,
  ...
}: let
  cfg = config.canix-toolbelt.hardware.gpu.intel.xe;
in {
  options.canix-toolbelt.hardware.gpu.intel.xe.forceProbeIds = lib.mkOption {
    type = lib.types.listOf lib.types.str;
    default = [];
    example = ["56a0"];
    description = "Intel PCI device IDs to force-bind to the xe DRM driver.";
  };

  config = lib.mkIf (cfg.forceProbeIds != []) ({
      boot = {
        initrd.kernelModules = ["xe"];
        kernelModules = ["xe"];
        kernelParams =
          map (id: "i915.force_probe=!${id}") cfg.forceProbeIds
          ++ map (id: "xe.force_probe=${id}") cfg.forceProbeIds;
        blacklistedKernelModules = ["i915"];
      };
    }
    // lib.optionalAttrs (options ? facter) {
      facter.detected.boot.graphics.kernelModules = lib.mkForce ["xe"];
    });
}
