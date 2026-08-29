{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  evaluated = lib.evalModules {
    modules = [
      ../modules/nixos/services/dev-agent-isolation.nix
      {
        options.systemd.user.slices = lib.mkOption {
          type = lib.types.attrs;
          default = {};
        };
        options.systemd.user.services = lib.mkOption {
          type = lib.types.attrs;
          default = {};
        };
        config.canix-toolbelt.services.devAgentIsolation = {
          enable = true;
          memoryHigh = "24G";
          memoryMax = "32G";
          memorySwapMax = "8G";
          cpuWeight = 50;
          ioWeight = 50;
        };
      }
    ];
    specialArgs = {inherit pkgs;};
  };

  sliceConfig = evaluated.config.systemd.user.slices.dev-agents.sliceConfig;
  serviceConfig = evaluated.config.systemd.user.services.dev-agent-workloads.serviceConfig;
in
  mkEvalCheck {
    name = "dev-agent-isolation-eval";
    resultMessage = "agent cgroup limits and attach anchor evaluated correctly";
    assertions = [
      {
        name = "slice-memory-limits";
        assertion =
          sliceConfig.MemoryHigh
          == "24G"
          && sliceConfig.MemoryMax == "32G"
          && sliceConfig.MemorySwapMax == "8G";
        message = "the agent slice must carry the configured memory and swap limits";
      }
      {
        name = "slice-scheduling-weights";
        assertion = sliceConfig.CPUWeight == "50" && sliceConfig.IOWeight == "50";
        message = "the agent slice must carry the configured CPU and I/O weights";
      }
      {
        name = "stable-service-anchor";
        assertion =
          serviceConfig.Type
          == "simple"
          && lib.hasInfix "/bin/sleep infinity" serviceConfig.ExecStart
          && serviceConfig.Delegate == true
          && serviceConfig.KillMode == "control-group"
          && serviceConfig.Restart == "on-failure"
          && serviceConfig.Slice == "dev-agents.slice";
        message = "the agent service must keep and restart a stable cgroup anchor";
      }
    ];
  }
