{
  config,
  lib,
  ...
}: let
  inherit (lib) mkOption types;
  cfg = config.canix-toolbelt.desktop.browserContainment;
in {
  options.canix-toolbelt.desktop.browserContainment = {
    enable = lib.mkEnableOption "browser memory containment and GPU guard";
    package = mkOption {
      type = types.nullOr types.package;
      default = null;
      description = "Package providing bin/browser-gpu-guard; required when enabled.";
    };
    memoryHigh = mkOption {
      type = types.str;
      default = "12G";
      description = "Memory reclaim threshold for app-browser.slice.";
    };
    memoryMax = mkOption {
      type = types.str;
      default = "16G";
      description = "Hard memory limit for app-browser.slice.";
    };
    memorySwapMax = mkOption {
      type = types.str;
      default = "2G";
      description = "Swap limit for app-browser.slice.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.package != null;
        message = "canix-toolbelt.desktop.browserContainment.package must provide bin/browser-gpu-guard when containment is enabled.";
      }
    ];

    systemd.oomd.enable = lib.mkDefault true;
    systemd.user.slices.app-browser.sliceConfig = {
      CPUAccounting = true;
      MemoryAccounting = true;
      MemoryHigh = cfg.memoryHigh;
      MemoryMax = cfg.memoryMax;
      MemorySwapMax = cfg.memorySwapMax;
      ManagedOOMMemoryPressure = "kill";
      ManagedOOMSwap = "kill";
    };

    systemd.user.services.canix-browser-gpu-guard = lib.mkIf (cfg.package != null) {
      description = "Browser GPU guard";
      wantedBy = ["graphical-session.target"];
      partOf = ["graphical-session.target"];
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/browser-gpu-guard";
        Restart = "on-failure";
        RestartSec = 5;
      };
    };
  };
}
