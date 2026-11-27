{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkIf mkEnableOption mkOption types;
  cfg = config.canix-toolbelt.rustdesk-client;
  presetConfig = pkgs.writeText "rustdesk-preset.toml" ''
    rendezvous_server = '${cfg.rendezvousServer}'
    nat_type = 1
    serial = 0

    [options]
    custom-rendezvous-server = '${cfg.rendezvousServer}'
    relay-server = '${cfg.rendezvousServer}'
  '';

  passwordScript = pkgs.writeShellScript "rustdesk-set-password" ''
    set -eu
    password_file="${cfg.passwordFile}"
    [ -r "$password_file" ] || { echo "password file not readable: $password_file"; exit 0; }
    password=$(cat "$password_file")
    rd_config="${config.xdg.configHome}/rustdesk/RustDesk.toml"
    [ -f "$rd_config" ] || { echo "RustDesk.toml not found — not yet enrolled"; exit 0; }
    device_id=$(${pkgs.gnused}/bin/sed -n 's/^id = "\([^"]*\)".*/\1/p' "$rd_config" 2>/dev/null || true)
    device_token=$(${pkgs.gnused}/bin/sed -n 's/^api_secret = "\([^"]*\)".*/\1/p' "$rd_config" 2>/dev/null || true)
    [ -n "$device_id" ] || { echo "device id not found in $rd_config"; exit 0; }
    [ -n "$device_token" ] || { echo "device token not found in $rd_config"; exit 0; }
    payload=$(${pkgs.jq}/bin/jq -n \
      --arg device_id "$device_id" \
      --arg device_token "$device_token" \
      --arg password "$password" \
      '{device_id: $device_id, device_token: $device_token, password: $password, unattended_enabled: true}')
    resp=$(${pkgs.curl}/bin/curl -s -o /dev/null -w "%{http_code}" \
      -X POST "http://${cfg.rendezvousServer}:${toString cfg.apiPort}/api/devices/self/access-policy" \
      -H "Content-Type: application/json" \
      -d "$payload")
    if [ "$resp" = "204" ]; then
      echo "BetterDesk password set successfully"
    else
      echo "BetterDesk API returned $resp" >&2
      exit 1
    fi
  '';
in {
  options.canix-toolbelt.rustdesk-client = {
    enable = mkEnableOption "RustDesk remote desktop client";

    usePassword = mkEnableOption "Set a permanent unattended password via the BetterDesk self-service API. Required for machines being controlled (targets).";

    package = mkOption {
      type = types.package;
      default = pkgs.rustdesk;
      defaultText = lib.literalMD "pkgs.rustdesk";
      description = "The RustDesk package to install.";
    };

    rendezvousServer = mkOption {
      type = types.str;
      description = "Hostname or IP of the BetterDesk/RustDesk rendezvous/relay server.";
      example = "10.123.0.1";
    };

    apiPort = mkOption {
      type = types.port;
      default = 21114;
      description = "HTTP port of the BetterDesk self-service API.";
    };

    passwordFile = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Path to a file containing the permanent plaintext password. When null, the password-setting systemd user service is not installed.";
    };
  };

  config = mkIf cfg.enable {
    home.packages = [cfg.package];

    home.activation.rustdesk-preset = lib.hm.dag.entryAfter ["writeBoundary"] ''
      rustdeskConfigDir="${config.xdg.configHome}/rustdesk"
      rustdeskConfigFile="$rustdeskConfigDir/RustDesk2.toml"
      if [ ! -e "$rustdeskConfigFile" ]; then
        run mkdir -p "$rustdeskConfigDir"
        run install -m 0644 ${presetConfig} "$rustdeskConfigFile"
      fi
    '';

    systemd.user.services.rustdesk-set-password = mkIf (cfg.usePassword && cfg.passwordFile != null) {
      Unit = {
        Description = "Set BetterDesk permanent password via self-service API";
        After = ["graphical-session.target"];
        BindsTo = ["graphical-session.target"];
      };

      Service = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${passwordScript}";
      };

      Install.WantedBy = ["default.target"];
    };
  };
}
