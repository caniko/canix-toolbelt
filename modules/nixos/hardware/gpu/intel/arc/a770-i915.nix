{
  config,
  lib,
  options,
  ...
}: {
  imports = [
    ./common.nix
  ];

  config = lib.mkIf (config.canix-toolbelt.hardware.gpu.profile == "intel-arc-a770-i915") ({
      boot = {
        initrd.kernelModules = ["i915"];
        kernelModules = ["i915"];
        blacklistedKernelModules = ["xe"];
      };
    }
    // lib.optionalAttrs (options ? facter) {
      facter.detected.boot.graphics.kernelModules = lib.mkForce ["i915"];
    });
}
