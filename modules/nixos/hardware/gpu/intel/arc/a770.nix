{
  config,
  lib,
  ...
}: {
  # Arc A770 / DG2-512: modern xe path with userspace pieces for gaming, media,
  # and compute workloads.
  imports = [
    ./common.nix
    ../xe.nix
  ];

  config = lib.mkIf (config.canix-toolbelt.hardware.gpu.profile == "intel-arc-a770-xe") {
    canix-toolbelt.hardware.gpu.intel.xe.forceProbeIds = ["56a0"];
  };
}
