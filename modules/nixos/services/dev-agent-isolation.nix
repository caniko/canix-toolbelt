{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.services.devAgentIsolation;
  sliceName = lib.removeSuffix ".slice" cfg.slice;
in {
  options.canix-toolbelt.services.devAgentIsolation = {
    enable = lib.mkEnableOption "a dedicated user cgroup for agent workloads";

    slice = lib.mkOption {
      type = lib.types.strMatching "[A-Za-z0-9_.@:-]+\\.slice";
      default = "dev-agents.slice";
      description = "User slice containing transient agent workload scopes.";
    };

    memoryHigh = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Optional soft memory limit for the aggregate agent workload slice.";
    };

    memoryMax = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Optional hard memory limit for the aggregate agent workload slice.";
    };

    memorySwapMax = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Optional swap limit for the aggregate agent workload slice.";
    };

    cpuWeight = lib.mkOption {
      type = lib.types.ints.between 1 10000;
      default = 50;
      description = "Relative CPU weight for agent workloads.";
    };

    ioWeight = lib.mkOption {
      type = lib.types.ints.between 1 10000;
      default = 50;
      description = "Relative block-I/O weight for agent workloads.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.user.slices.${sliceName}.sliceConfig =
      {
        MemoryAccounting = true;
        CPUAccounting = true;
        IOAccounting = true;
        CPUWeight = toString cfg.cpuWeight;
        IOWeight = toString cfg.ioWeight;
      }
      // lib.optionalAttrs (cfg.memoryHigh != null) {MemoryHigh = cfg.memoryHigh;}
      // lib.optionalAttrs (cfg.memoryMax != null) {MemoryMax = cfg.memoryMax;}
      // lib.optionalAttrs (cfg.memorySwapMax != null) {MemorySwapMax = cfg.memorySwapMax;};
  };
}
