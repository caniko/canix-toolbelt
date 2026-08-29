{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.services.devAgentIsolation;
  sliceName = lib.removeSuffix ".slice" cfg.slice;
  unitName = lib.removeSuffix ".service" cfg.unit;
in {
  options.canix-toolbelt.services.devAgentIsolation = {
    enable = lib.mkEnableOption "a dedicated user cgroup for agent workloads";

    unit = lib.mkOption {
      type = lib.types.strMatching "[A-Za-z0-9_.@:-]+\\.service";
      default = "dev-agent-workloads.service";
      description = "User service whose cgroup receives attached agent processes.";
    };

    slice = lib.mkOption {
      type = lib.types.strMatching "[A-Za-z0-9_.@:-]+\\.slice";
      default = "dev-agents.slice";
      description = "User slice containing the agent workload service.";
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

    systemd.user.services.${unitName} = {
      description = "Cgroup anchor for agent descendants";
      wantedBy = ["default.target"];
      serviceConfig = {
        # Keep an anchor process alive so AttachProcessesToUnit has a stable
        # service cgroup before the first agent attaches.
        Type = "simple";
        ExecStart = "${pkgs.coreutils}/bin/sleep infinity";
        Slice = cfg.slice;
        Delegate = true;
        KillMode = "control-group";
        OOMPolicy = "continue";
        Restart = "on-failure";
      };
    };
  };
}
