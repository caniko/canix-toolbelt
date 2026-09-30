# Runtime glue for a caller-provided frontend using Fleetix's health library.
{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkOption types;
  cfg = config.services.gatusHealthPublisher;
  checks = builtins.filter (c: (c.host or c.source.probe.host) == config.networking.hostName) cfg.checks;
  manifest = (pkgs.formats.json {}).generate "gatus-health-publisher.json" {
    host = config.networking.hostName;
    inherit (cfg) url;
    systemctl = "${pkgs.systemd}/bin/systemctl";
    timeout = "${pkgs.coreutils}/bin/timeout";
    checks = map (c: {inherit (c) key source;} // lib.optionalAttrs (c ? target) {inherit (c) target;}) checks;
  };
  intervals = map (check: check.source.intervalSeconds or 60) checks;
  minInterval =
    if intervals == []
    then 60
    else lib.foldl' lib.min (builtins.head intervals) intervals;
  interval = lib.min minInterval 60;
  # Fleetix collects and publishes in batches of eight. Include kill-after and
  # timer accuracy so a maximum-duration run finishes before heartbeat expiry.
  batches = builtins.div (builtins.length checks + 7) 8;
  probeTimeout = lib.foldl' (value: check: lib.max value (check.source.timeoutSeconds or 10)) 1 checks;
  runBudget = batches * (probeTimeout + 1 + 10) + 5;
in {
  options.services.gatusHealthPublisher = {
    enable = lib.mkEnableOption "host-local health publication";
    command = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "Publisher frontend command; receives --config and --credential-env-file. The caller chooses its Fleetix-based frontend.";
    };
    checks = mkOption {
      type = types.listOf types.attrs;
      default = [];
      description = "Fleetix Gatus externalChecks; filtered to this host.";
    };
    url = mkOption {
      type = types.str;
      description = "Internal Gatus publication origin (loopback or an authenticated private ingress).";
    };
    credentialEnvFile = mkOption {
      type = types.str;
      description = "Runtime file containing GATUS_EXTERNAL_TOKEN, loaded through systemd credentials.";
    };
  };
  config = lib.mkIf (cfg.enable && checks != []) {
    assertions = [
      {
        assertion = cfg.command != [];
        message = "gatusHealthPublisher requires a publisher frontend command.";
      }
      {
        assertion = interval >= 10;
        message = "gatusHealthPublisher polling intervals must be at least ten seconds.";
      }
      {
        assertion = runBudget + interval + 5 < 3 * minInterval;
        message = "gatusHealthPublisher run limit exceeds the heartbeat budget; increase check intervals or split the publisher.";
      }
    ];
    systemd.services.gatus-health-publisher = {
      description = "Publish host-local service health";
      wants = ["network-online.target"];
      after = ["network-online.target"];
      serviceConfig = {
        Type = "oneshot";
        DynamicUser = true;
        LoadCredential = ["health.env:${cfg.credentialEnvFile}"];
        ExecStart = lib.escapeShellArgs (cfg.command ++ ["--config" (toString manifest) "--credential-env-file" "%d/health.env"]);
        TimeoutStartSec = "${toString runBudget}s";
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectHome = true;
        ProtectSystem = "strict";
        RestrictAddressFamilies = ["AF_INET" "AF_INET6" "AF_UNIX"];
      };
    };
    systemd.timers.gatus-health-publisher = {
      wantedBy = ["timers.target"];
      timerConfig = {
        OnBootSec = "2min";
        OnUnitInactiveSec = "${toString interval}s";
        AccuracySec = "5s";
      };
    };
  };
}
