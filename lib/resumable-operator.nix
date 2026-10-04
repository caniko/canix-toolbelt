{lib}: {
  pkgs,
  name,
  stages,
  stateDir,
  requestPath ? "${stateDir}/requested",
  sentinelPath ? "${stateDir}/running",
  maxAttempts ? 1,
  retryDelays ? [],
  quiesceUnits ? [],
  resumeOnBoot ? false,
  description ? "${name} resumable operator",
  after ? [],
  requires ? [],
  wants ? [],
  controllerPackage ? import ../nix/package.nix {inherit pkgs;},
  # The helper cannot infer commands of consumer-owned systemd units.
  # Callers must bind executable paths, arguments and data roots explicitly.
  executionContract,
  extraServiceConfig ? {},
}:
assert stages != [];
assert builtins.isString executionContract && executionContract != "";
assert maxAttempts > 0;
assert builtins.length retryDelays >= maxAttempts - 1; let
  # Nix supplies policy; the Rust engine owns locking, durable checkpoints,
  # retries, cancellation, worker recovery and reports.
  policy = pkgs.writeText "${name}-policy.json" (builtins.toJSON {
    inherit name stages;
    contract_id = executionContract;
    state_dir = stateDir;
    request_path = requestPath;
    sentinel_path = sentinelPath;
    max_attempts = maxAttempts;
    retry_delays = retryDelays;
    quiesce_units = quiesceUnits;
  });
  command = ["${controllerPackage}/bin/canix-toolbelt" "operator"];
  policyArgs = ["--config" (toString policy)];
  mainService = {
    inherit description after requires wants;
    unitConfig = {
      RequiresMountsFor = stateDir;
      StartLimitBurst = 5;
      StartLimitIntervalSec = "1h";
    };
    serviceConfig =
      {
        Type = "notify";
        User = "root";
        Group = "root";
        ExecStart = lib.escapeShellArgs (command
          ++ ["run"]
          ++ policyArgs
          ++ [
            "--systemctl"
            "${pkgs.systemd}/bin/systemctl"
            "--systemd-notify"
            "${pkgs.systemd}/bin/systemd-notify"
          ]);
        Restart = "on-failure";
        RestartSec = "2min";
        RestartPreventExitStatus = "20";
        NotifyAccess = "all";
        TimeoutStopSec = "30min";
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        NoNewPrivileges = true;
        RestrictAddressFamilies = ["AF_UNIX"];
        ReadWritePaths = [stateDir (builtins.dirOf sentinelPath)];
        UMask = "0027";
      }
      // extraServiceConfig;
  };
  cancelService = {
    description = "Cancel ${description}";
    conflicts = ["${name}.service"];
    before = ["${name}.service"];
    unitConfig.RequiresMountsFor = stateDir;
    serviceConfig = {
      Type = "oneshot";
      ExecStart = lib.escapeShellArgs (command ++ ["cancel"] ++ policyArgs);
    };
  };
  resumeService = {
    description = "Resume an interrupted ${description}";
    wants = ["network-online.target"];
    after = ["network-online.target"] ++ after;
    wantedBy = ["multi-user.target"];
    unitConfig = {
      ConditionPathExists = requestPath;
      RequiresMountsFor = stateDir;
    };
    serviceConfig = {
      Type = "oneshot";
      ExecStart = "${pkgs.systemd}/bin/systemctl start --no-block ${name}.service";
    };
  };
in {
  systemd.services =
    {
      "${name}" = mainService;
      "${name}-cancel" = cancelService;
    }
    // lib.optionalAttrs resumeOnBoot {
      "${name}-resume" = resumeService;
    };
}
