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
  wrapper = (import ../lib/dev-agent-isolation.nix {inherit lib;}).mkScopeExecWrapper {
    inherit pkgs;
    name = "dev-agent-scope-eval";
    slice = "dev-agents.slice";
    targetPath = "${pkgs.coreutils}/bin/true";
  };
in
  mkEvalCheck {
    name = "dev-agent-isolation-eval";
    resultMessage = "agent slice limits and scope wrapper evaluated correctly";
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
        name = "no-attach-anchor";
        assertion = !(evaluated.config.systemd.user.services ? dev-agent-workloads);
        message = "the retired attach-anchor service must not be generated";
      }
    ];
    runtimeScript = ''
      grep -F -- '--scope' ${wrapper}
      grep -F -- '--quiet' ${wrapper}
      grep -F -- '--collect' ${wrapper}
      grep -F -- '--same-dir' ${wrapper}
      grep -F -- '--expand-environment=no' ${wrapper}
      grep -F 'dev-agents.slice' ${wrapper}
      grep -F systemd-run ${wrapper}
      if grep -F AttachProcessesToUnit ${wrapper}; then
        echo "scope wrapper still contains AttachProcessesToUnit" >&2
        exit 1
      fi
      if grep -F 'sleep infinity' ${wrapper}; then
        echo "scope wrapper still contains sleep infinity" >&2
        exit 1
      fi
    '';
  }
