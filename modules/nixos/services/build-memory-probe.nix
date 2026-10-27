# Root sampler that watches a cgroup-v2 memory domain and appends one JSONL
# "episode" per heavy build to `<reportDir>/episodes-YYYY-MM.jsonl`: sampled
# peak, swap peak, memory.high/max hits, OOM kills, PSI stall time and a
# hit_ceiling flag. A running episode is checkpointed to active-episode.json
# and archived as interrupted if the sampler restarts mid-episode.
{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkOption types;
  cfg = config.canix-toolbelt.services.buildMemoryProbe;
  activation = import ../../../lib/activation.nix {inherit lib;};
  probe = import ../../../nix/build-memory-probe.nix {inherit pkgs;};
  memoryFiles = ["memory.current" "memory.events" "memory.pressure" "memory.high" "memory.max"];
  readable = dir: lib.concatMapStringsSep " && " (file: "test -r ${dir}/${file}") memoryFiles;

  activationContract = activation.mkContract {
    name = "canix-toolbelt.services.buildMemoryProbe";
    enabled = cfg.enable;
    requirements.cgroup = {
      phase = "runtime";
      satisfied = false;
      summary = "The sampled cgroup-v2 memory files must be available before the probe can sample builds.";
      producer = "Linux kernel cgroup-v2 / Nix daemon service";
      recovery = "Boot a cgroup-v2-enabled host with nix-daemon.service active, then restart the probe service.";
      validation =
        if cfg.cgroup != ""
        then readable cfg.cgroup
        else ''
          cg="$(${pkgs.systemd}/bin/systemctl show nix-daemon.service --property=ControlGroup --value)"
          test -n "$cg" && ${readable "\"/sys/fs/cgroup$cg\""}
        '';
    };
    artifacts = [
      {
        kind = "directory";
        path = cfg.reportDir;
        sensitive = false;
      }
    ];
    units = ["build-memory-probe.service"];
  };
in {
  imports = [../activation-contracts.nix];

  options.canix-toolbelt.services.buildMemoryProbe = {
    enable = lib.mkEnableOption ''
      a lightweight root sampler that logs one JSONL episode record per heavy
      build. Pairs with a cgroup MemoryHigh/MemoryMax guard so the masked
      memory demand stays observable'';

    cgroup = mkOption {
      type = types.str;
      default = "";
      example = "/sys/fs/cgroup/system.slice/nix-daemon.service";
      description = ''
        Absolute cgroup directory to sample. Empty samples nix-daemon.service's
        live ControlGroup, or its delegated `nix-builders` child when
        `nix.settings.build-cgroup-memory-max` is set.
      '';
    };

    reportDir = mkOption {
      type = types.str;
      default = "/var/log/build-memory";
      description = "Directory for episode, checkpoint and interrupted-episode records.";
    };

    intervalSec = mkOption {
      type = types.ints.positive;
      default = 3;
      description = "Sampling interval in seconds.";
    };

    busyThresholdBytes = mkOption {
      type = types.ints.positive;
      example = 8 * 1024 * 1024 * 1024;
      description = ''
        memory.current at or above which an episode starts. memory.high,
        OOM-kill and full-PSI increases also start one. Host-specific: set it
        below the host's build memory cap.
      '';
    };

    idleThresholdBytes = mkOption {
      type = types.ints.positive;
      example = 2 * 1024 * 1024 * 1024;
      description = "memory.current below which a sample counts as idle.";
    };

    idleDebounceTicks = mkOption {
      type = types.ints.positive;
      default = 3;
      description = "Consecutive idle samples that end an episode.";
    };
  };

  config = lib.mkMerge [
    {
      inherit (activationContract) assertions;
      canix-toolbelt.activation.contracts.buildMemoryProbe = lib.removeAttrs activationContract ["name" "assertions" "status" "declaredStatus"];
    }

    (lib.mkIf cfg.enable {
      systemd.tmpfiles.rules = ["d ${cfg.reportDir} 0755 root root -"];

      systemd.services.build-memory-probe = {
        description = "Sample build cgroup memory pressure; log heavy-build episodes to ${cfg.reportDir}";
        wantedBy = ["multi-user.target"];
        after = ["nix-daemon.service"];
        partOf = ["nix-daemon.service"];
        environment = {
          BUILD_MEMORY_PROBE_CGROUP = cfg.cgroup;
          BUILD_MEMORY_PROBE_REPORT_DIR = cfg.reportDir;
          BUILD_MEMORY_PROBE_INTERVAL_SEC = toString cfg.intervalSec;
          BUILD_MEMORY_PROBE_BUSY_BYTES = toString cfg.busyThresholdBytes;
          BUILD_MEMORY_PROBE_IDLE_BYTES = toString cfg.idleThresholdBytes;
          BUILD_MEMORY_PROBE_IDLE_DEBOUNCE_TICKS = toString cfg.idleDebounceTicks;
          BUILD_MEMORY_PROBE_HOST = config.networking.hostName;
          BUILD_MEMORY_PROBE_AGGREGATE_BUILDERS =
            if (config.nix.settings.build-cgroup-memory-max or 0) > 0
            then "1"
            else "0";
        };
        serviceConfig = {
          Type = "simple";
          ExecStart = lib.getExe probe;
          Restart = "always";
          RestartSec = 5;
          Slice = "diagnostics.slice";
          # The probe must never itself contribute to the pressure it measures.
          Nice = 19;
          CPUSchedulingPolicy = "idle";
          IOSchedulingClass = "idle";
          MemoryHigh = "infinity";
          MemoryMax = "128M";
        };
      };
      systemd.slices.diagnostics = {};
    })
  ];
}
