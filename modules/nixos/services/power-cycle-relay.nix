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
    hasWol = (host.macAddress or null) != null && (host.lanBroadcast or null) != null;
    wolCmd =
      if hasWol
      then ''
        echo "Sending Wake-on-LAN to ${name}..."
        ${pkgs.wol}/bin/wol -i ${host.lanBroadcast} ${host.macAddress}
      ''
      else ''echo "No WoL data for ${name}, skipping wake"'';
  in
    pkgs.writeShellScript "power-cycle-${name}" ''
      set -euo pipefail
      TOKEN=$(cat "${cfg.haTokenFile}")

      echo "Turning off ${name} via Home Assistant..."
      ${pkgs.curl}/bin/curl -sf -X POST "${cfg.haUrl}/api/services/switch/turn_off" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d '{"entity_id": "${target.entityId}"}'

      echo "Waiting 10s for power drain..."
      sleep 10

      echo "Turning on ${name} via Home Assistant..."
      ${pkgs.curl}/bin/curl -sf -X POST "${cfg.haUrl}/api/services/switch/turn_on" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d '{"entity_id": "${target.entityId}"}'

      echo "Waiting 30s for POST..."
      sleep 30

      ${wolCmd}
      echo "Power cycle of ${name} complete."
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
