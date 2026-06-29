{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.services.power-cycle-relay;
  allHosts = config.canix-toolbelt.hosts;
  inherit (lib) mapAttrs' mkEnableOption mkIf mkOption types;

  mkPowerCycleScript = name: target: let
    host = allHosts.${name} or {};
    canixBin = "${pkgs.canix}/bin/canix";
  in
    pkgs.writeShellScript "power-cycle-${name}" ''
      exec ${canixBin} power-cycle \
        "${name}" \
        --ha-url "${cfg.haUrl}" \
        --ha-token-file "${cfg.haTokenFile}" \
        ${lib.optionalString ((host.macAddress or null) != null) ''--mac "${host.macAddress}"''}
    '';
in {
  options.canix-toolbelt.services.power-cycle-relay = {
    enable = mkEnableOption "remote power cycling via Home Assistant smart plugs";

    targets = mkOption {
      type = types.attrsOf (types.submodule {
        options.entityId = mkOption {
          type = types.str;
          description = "Home Assistant switch entity ID (e.g. switch.atlas_power)";
        };
      });
      default = {};
    };

    haUrl = mkOption {
      type = types.str;
      default = "http://127.0.0.1:8123";
      description = "Home Assistant API URL";
    };

    haTokenFile = mkOption {
      type = types.str;
      description = "Path to file containing HA long-lived access token";
    };
  };

  config = mkIf (cfg.enable && cfg.targets != {}) {
    environment.systemPackages = [pkgs.curl];

    systemd.services =
      mapAttrs' (name: target: {
        name = "power-cycle-${name}";
        value = {
          description = "Power cycle ${name} via Home Assistant smart plug";
          serviceConfig = {
            Type = "oneshot";
            ExecStart = mkPowerCycleScript name target;
          };
        };
      })
      cfg.targets;
  };
}
