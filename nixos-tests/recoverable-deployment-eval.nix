{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  # Eval fixtures only. These packages are never executed by this check.
  dummyPackage = pkgs.writeTextFile {
    name = "fleetix-deploy-eval-dummy";
    destination = "/bin/fleetix-deploy";
    executable = true;
    text = ''
      #!${pkgs.runtimeShell}
      echo "eval dummy: fleetix-deploy is not executed by this check" >&2
      exit 1
    '';
  };
  verifier = pkgs.writeTextFile {
    name = "fleetix-admission-eval-dummy";
    destination = "/bin/admit";
    executable = true;
    text = ''
      #!${pkgs.runtimeShell}
      exit 1
    '';
  };
  health = pkgs.writeTextFile {
    name = "fleetix-health-eval-dummy";
    destination = "/bin/check";
    executable = true;
    text = ''
      #!${pkgs.runtimeShell}
      exit 1
    '';
  };

  evaluateNixos = overrides:
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      system = pkgs.stdenv.hostPlatform.system;
      modules = [
        ../modules/nixos/recoverable-deployment.nix
        {
          system.stateVersion = "24.11";
          boot.isContainer = true;
          fileSystems."/" = {
            device = "none";
            fsType = "tmpfs";
          };
        }
        overrides
      ];
    }).config;

  nixosBase = {
    enable = true;
    package = dummyPackage;
    requestPath = "/var/lib/fleetix/requests/atlas.json";
    admissionCommand = ["${verifier}/bin/admit" "--trusted"];
    healthCommand = ["${health}/bin/check"];
  };
  nixosDeployment = overrides:
    evaluateNixos {
      services.recoverable-deployment.deployments.atlas = nixosBase // overrides;
    };
  enabled = nixosDeployment {};

  runName = "recoverable-deployment-atlas";
  recoverName = "recoverable-deployment-atlas-recover";
  deployment = enabled.services.recoverable-deployment.deployments.atlas;
  inherit (deployment) policy;
  run = enabled.systemd.services.${runName};
  recover = enabled.systemd.services.${recoverName};
  timer = enabled.systemd.timers.${recoverName};
  startupTimer = enabled.systemd.timers.${runName};
  pathUnit = enabled.systemd.paths.${runName};
  expectedTimeout = lib.max 7200 (16 * deployment.commandTimeoutSeconds + 3 * deployment.healthTimeoutSeconds + 180);
  expectedRun = lib.escapeShellArgs [
    "${dummyPackage}/bin/fleetix-deploy"
    "run"
    "--policy"
    deployment.policyFile
    "--request"
    nixosBase.requestPath
  ];
  expectedRecover = lib.escapeShellArgs [
    "${dummyPackage}/bin/fleetix-deploy"
    "recover"
    "--policy"
    deployment.policyFile
  ];

  disabled = evaluateNixos {
    services.recoverable-deployment.deployments.atlas = {enable = false;};
  };
  pristine = evaluateNixos {};
  watchOff = nixosDeployment {watchRequest = false;};
  customLock = nixosDeployment {lockPath = "/run/canix/deployment.lock";};
  missingPackage = nixosDeployment {package = null;};
  missingCommands = nixosDeployment {
    admissionCommand = [];
    healthCommand = [];
  };
  relativeCommand = nixosDeployment {admissionCommand = ["admit"];};
  customTimeouts = nixosDeployment {
    commandTimeoutSeconds = 3600;
    healthTimeoutSeconds = 3600;
  };
  timeoutRejected = builtins.tryEval (nixosDeployment {commandTimeoutSeconds = 3601;}).systemd.services.${runName}.serviceConfig.TimeoutStartSec;
  activeAssertions = assertions:
    lib.filter (entry: !entry.assertion && lib.hasPrefix "Recoverable deployment 'atlas'" entry.message) assertions;

  submissionFixture = overrides:
    evaluateNixos {
      users.users.fixture.isNormalUser = true;
      services.recoverable-deployment.deployments.atlas =
        (builtins.removeAttrs nixosBase ["requestPath"])
        // {requestUsers = ["fixture"];}
        // overrides;
    };
  submission = submissionFixture {};
  submissionConfig = submission.services.recoverable-deployment.deployments.atlas;
  submissionGroup = "fleetix-${builtins.substring 0 24 (builtins.hashString "sha256" "atlas")}";
  noSubmissionWatch = submissionFixture {watchRequest = false;};
  unsafeSubmissionPath = submissionFixture {requestPath = "/etc/request.json";};
  unknownSubmitter = submissionFixture {requestUsers = ["undeclared-user"];};

  evaluateHome = overrides:
    (inputs.home-manager.lib.homeManagerConfiguration {
      inherit pkgs;
      modules = [
        ../modules/home/recoverable-deployment.nix
        {
          home = {
            username = "fixture";
            homeDirectory = "/home/fixture";
            stateVersion = "25.05";
          };
        }
        overrides
      ];
    }).config;
  homeBase = {
    enable = true;
    package = dummyPackage;
    requestPath = "/home/fixture/.local/state/fleetix-deployment/requests/atlas.json";
    admissionCommand = ["${verifier}/bin/admit"];
    healthCommand = ["${health}/bin/check"];
  };
  homeDeployment = overrides:
    evaluateHome {
      services.recoverable-deployment.deployments.atlas = homeBase // overrides;
    };
  homeEnabled = homeDeployment {};
  homeDeploymentConfig = homeEnabled.services.recoverable-deployment.deployments.atlas;
  homePolicy = homeDeploymentConfig.policy;
  homeRun = homeEnabled.systemd.user.services.${runName};
  homeRecover = homeEnabled.systemd.user.services.${recoverName};
  homeTimer = homeEnabled.systemd.user.timers.${recoverName};
  homePath = homeEnabled.systemd.user.paths.${runName};
  homeExpectedTimeout = lib.max 7200 (16 * homeDeploymentConfig.commandTimeoutSeconds + 3 * homeDeploymentConfig.healthTimeoutSeconds + 180);
  homeDisabled = evaluateHome {
    services.recoverable-deployment.deployments.atlas = {enable = false;};
  };
  homeWatchOff = homeDeployment {watchRequest = false;};
  homeOutside = homeDeployment {stateRoot = "/var/lib/fleetix/deployment/atlas";};
  homeMissing = homeDeployment {package = null;};
  homeNoAdmission = homeDeployment {admissionCommand = [];};
  homeNoHealth = homeDeployment {healthCommand = [];};
in
  mkEvalCheck {
    name = "recoverable-deployment-eval";
    resultMessage = "recoverable-deployment eval assertions passed; dummy packages, no live worker";
    assertions = [
      {
        name = "disabled-default";
        assertion =
          !(disabled.systemd.services ? ${runName})
          && !(disabled.systemd.services ? ${recoverName})
          && !(disabled.systemd.timers ? ${recoverName})
          && !(disabled.systemd.timers ? ${runName})
          && !(disabled.systemd.paths ? ${runName})
          && !(lib.any (rule: lib.hasInfix disabled.services.recoverable-deployment.deployments.atlas.stateRoot rule) disabled.systemd.tmpfiles.rules);
        message = "A deployment must add no units or state rules until explicitly enabled.";
      }
      {
        name = "explicit-package";
        assertion = deployment.package == dummyPackage && deployment.policyFile != "" && lib.hasPrefix "/nix/store/" deployment.policyFile;
        message = "An enabled deployment must use the explicit package and a store-backed policy.";
      }
      {
        name = "policy-schema";
        assertion =
          policy.version
          == 1
          && policy.target == "atlas"
          && policy.backend == "nixos"
          && policy.stateRoot == "/var/lib/fleetix/deployment/atlas"
          && policy.lockPath == "/run/lock/fleetix-deployment.lock"
          && policy.profilePath == "/nix/var/nix/profiles/system"
          && policy.currentSystemPath == "/run/current-system"
          && lib.hasSuffix "/bin/nix-store" policy.nixStore
          && lib.hasSuffix "/bin/nix-env" policy.nixEnv
          && policy.admissionCommand == ["${verifier}/bin/admit" "--trusted"]
          && policy.healthCommand == ["${health}/bin/check"]
          && !(policy ? approved);
        message = "The NixOS policy must bind fixed target, backend, paths and trusted commands without an approval flag.";
      }
      {
        name = "mandatory-trusted-commands";
        assertion =
          lib.any (entry: !entry.assertion && lib.hasInfix "explicit package" entry.message) (activeAssertions missingPackage.assertions)
          && lib.any (entry: !entry.assertion && lib.hasInfix "admission command" entry.message) (activeAssertions missingCommands.assertions)
          && lib.any (entry: !entry.assertion && lib.hasInfix "health command" entry.message) (activeAssertions missingCommands.assertions)
          && lib.any (entry: !entry.assertion && lib.hasInfix "admission command" entry.message) (activeAssertions relativeCommand.assertions);
        message = "Package, admission verifier and health command must be explicit and absolute.";
      }
      {
        name = "root-run-unit";
        assertion =
          run.serviceConfig.Type
          == "oneshot"
          && run.serviceConfig.ExecStart == expectedRun
          && run.serviceConfig.KillMode == "control-group"
          && run.serviceConfig.TimeoutStartSec == expectedTimeout
          && !run.restartIfChanged
          && run.wantedBy == []
          && run.onFailure == ["${recoverName}.service"]
          && !(run.serviceConfig ? User)
          && !(run.serviceConfig ? DynamicUser)
          && !(run.serviceConfig ? NoNewPrivileges)
          && !(run.serviceConfig ? ProtectSystem)
          && !(run.serviceConfig ? ProtectHome)
          && !(run.serviceConfig ? PrivateTmp)
          && !(run.serviceConfig ? RuntimeDirectory)
          && !(run.serviceConfig ? ExecStopPost)
          && !(run.serviceConfig ? Restart)
          && !(run.serviceConfig ? SuccessExitStatus)
          && run.serviceConfig.RestartForceExitStatus == [75]
          && run.serviceConfig.RestartSec == "30s";
        message = "The root run service must be a bounded oneshot with fixed policy/request arguments and no privilege or sandbox switch.";
      }
      {
        name = "root-recovery-linkage";
        assertion =
          recover.serviceConfig.Type
          == "oneshot"
          && recover.serviceConfig.ExecStart == expectedRecover
          && recover.serviceConfig.KillMode == "control-group"
          && recover.serviceConfig.SuccessExitStatus == [75]
          && recover.serviceConfig.TimeoutStartSec == expectedTimeout
          && !recover.restartIfChanged
          && !(recover.serviceConfig ? ExecStopPost)
          && !(recover.serviceConfig ? RuntimeDirectory)
          && recover.wantedBy == []
          && lib.elem "local-fs.target" recover.after
          && timer.wantedBy == ["timers.target"]
          && timer.timerConfig.OnBootSec == "30s"
          && timer.timerConfig.OnUnitInactiveSec == "60s"
          && timer.timerConfig.Unit == "${recoverName}.service";
        message = "OnFailure, boot and periodic recovery must share the fixed-policy recover service.";
      }
      {
        name = "state-and-lock";
        assertion =
          enabled.systemd.tmpfiles.rules == pristine.systemd.tmpfiles.rules;
        message = "The owner-checking worker, not chmodding tmpfiles rules, must create private state and missing lock anchors.";
      }
      {
        name = "legacy-lock";
        assertion =
          customLock.services.recoverable-deployment.deployments.atlas.policy.lockPath
          == "/run/canix/deployment.lock"
          && !(lib.any (rule: lib.hasInfix "/run/canix" rule) customLock.systemd.tmpfiles.rules);
        message = "A legacy shared lock must be selected without changing its owner-provisioned anchor or parent.";
      }
      {
        name = "path-watch";
        assertion =
          pathUnit.pathConfig.PathChanged
          == nixosBase.requestPath
          && pathUnit.pathConfig.Unit == "${runName}.service"
          && pathUnit.wantedBy == ["paths.target"]
          && run.unitConfig.ConditionPathExists == nixosBase.requestPath
          && startupTimer.wantedBy == ["timers.target"]
          && startupTimer.timerConfig.OnBootSec == "30s"
          && startupTimer.timerConfig.Unit == "${runName}.service"
          && !(watchOff.systemd.timers ? ${runName})
          && !(watchOff.systemd.paths ? ${runName})
          && watchOff.systemd.services ? ${runName};
        message = "The request path watch must be optional while leaving the run service available.";
      }
      {
        name = "no-sudo-or-polkit";
        assertion =
          enabled.security.sudo.enable
          == pristine.security.sudo.enable
          && enabled.security.sudo.extraRules == pristine.security.sudo.extraRules
          && enabled.security.polkit.enable == pristine.security.polkit.enable
          && enabled.security.polkit.extraConfig == pristine.security.polkit.extraConfig
          && !(lib.hasInfix "sudo" run.serviceConfig.ExecStart)
          && !(lib.hasInfix "pkexec" run.serviceConfig.ExecStart);
        message = "The module must not add sudo or polkit escalation.";
      }
      {
        name = "timeout-bounds";
        assertion =
          expectedTimeout
          >= 16 * deployment.commandTimeoutSeconds + 3 * deployment.healthTimeoutSeconds + 180
          && expectedTimeout >= 7200
          && customTimeouts.systemd.services.${runName}.serviceConfig.TimeoutStartSec == 68580
          && !timeoutRejected.success;
        message = "The service timeout must follow the configured worker budget and reject out-of-range timeouts.";
      }
      {
        name = "home-disabled-default";
        assertion =
          !(homeDisabled.systemd.user.services ? ${runName})
          && !(homeDisabled.systemd.user.services ? ${recoverName})
          && !(homeDisabled.systemd.user.timers ? ${recoverName})
          && !(homeDisabled.systemd.user.paths ? ${runName})
          && !(homeDisabled.home.activation ? "recoverable-deployment-atlas");
        message = "A disabled Home Manager deployment must add no user units, timer, path or activation.";
      }
      {
        name = "home-policy";
        assertion =
          homePolicy.version
          == 1
          && homePolicy.target == "atlas"
          && homePolicy.backend == "home-manager"
          && homePolicy.stateRoot == "/home/fixture/.local/state/fleetix-deployment/atlas"
          && homePolicy.lockPath == "/home/fixture/.local/state/fleetix-deployment/deployment.lock"
          && homePolicy.profilePath == "/home/fixture/.local/state/nix/profiles/home-manager"
          && homePolicy.currentSystemPath == homePolicy.profilePath
          && lib.hasSuffix "/bin/nix-store" homePolicy.nixStore
          && lib.hasSuffix "/bin/nix-env" homePolicy.nixEnv
          && homePolicy.admissionCommand == ["${verifier}/bin/admit"]
          && homePolicy.healthCommand == ["${health}/bin/check"]
          && !(homePolicy ? approved);
        message = "The Home Manager policy must observe and restore the same fixed profile.";
      }
      {
        name = "home-units";
        assertion =
          homeRun.Unit.OnFailure
          == ["${recoverName}.service"]
          && homeRun.Service.Type == "oneshot"
          && homeRun.Service.KillMode == "control-group"
          && homeRun.Service.RestartForceExitStatus == [75]
          && homeRun.Service.RestartSec == "30s"
          && !(homeRun.Service ? Restart)
          && !(homeRun.Service ? SuccessExitStatus)
          && homeRun.Service.TimeoutStartSec == homeExpectedTimeout
          && !(homeRun.Service ? User)
          && !(homeRun.Service ? DynamicUser)
          && homeRecover.Install.WantedBy == ["default.target"]
          && homeRecover.Service.SuccessExitStatus == [75]
          && homeTimer.Install.WantedBy == ["timers.target"]
          && homeTimer.Timer.OnBootSec == "30s"
          && homeTimer.Timer.OnUnitInactiveSec == "60s"
          && homeTimer.Timer.Unit == "${recoverName}.service"
          && !(lib.hasInfix "graphical" (builtins.toJSON [homeRun homeRecover homeTimer homePath]));
        message = "Home Manager units must use default.target and a bounded oneshot recovery timer.";
      }
      {
        name = "home-activation";
        assertion = !(homeEnabled.home.activation ? "recoverable-deployment-atlas");
        message = "Home Manager must not chmod pre-existing state or lock parents; the worker validates ownership.";
      }
      {
        name = "home-path-and-home-boundary";
        assertion =
          homePath.Path.PathChanged
          == homeBase.requestPath
          && homePath.Path.Unit == "${runName}.service"
          && homePath.Install.WantedBy == ["paths.target"]
          && !(homeWatchOff.systemd.user.paths ? ${runName})
          && homeWatchOff.systemd.user.services ? ${runName}
          # Home Manager rejects invalid configurations before exposing config.
          && !(builtins.tryEval homeOutside.assertions).success
          && !(builtins.tryEval homeMissing.assertions).success
          && !(builtins.tryEval homeNoAdmission.assertions).success
          && !(builtins.tryEval homeNoHealth.assertions).success;
        message = "The user path watch is optional and every writable user path stays inside home.";
      }
      {
        name = "all-enabled-assertions";
        assertion = lib.all (entry: entry.assertion) enabled.assertions && lib.all (entry: entry.assertion) homeEnabled.assertions;
        message = "Enabled NixOS and Home Manager fixtures must satisfy every generated assertion.";
      }
      {
        name = "no-sudo-submission";
        assertion =
          submissionConfig.requestPath
          == "/var/lib/fleetix-requests/atlas/request.json"
          && submission.users.groups.${submissionGroup}.members == ["fixture"]
          && lib.elem "d /var/lib/fleetix-requests/atlas 2770 root ${submissionGroup} - -" submission.systemd.tmpfiles.rules
          && submission.systemd.paths.${runName}.pathConfig.PathChanged == submissionConfig.requestPath
          && submissionConfig.policy == policy
          && submission.security.sudo.extraRules == pristine.security.sudo.extraRules
          && submission.security.polkit.extraConfig == pristine.security.polkit.extraConfig
          && lib.all (entry: entry.assertion) submission.assertions;
        message = "Named users may nominate requests through a dedicated inbox, never bypassing the fixed policy or receiving sudo/polkit authority.";
      }
      {
        name = "submission-boundary";
        assertion =
          lib.any (entry: lib.hasInfix "request watch" entry.message) (activeAssertions noSubmissionWatch.assertions)
          && lib.any (entry: lib.hasInfix "managed request inbox" entry.message) (activeAssertions unsafeSubmissionPath.assertions)
          && lib.any (entry: lib.hasInfix "declared users" entry.message) (activeAssertions unknownSubmitter.assertions);
        message = "No-sudo submission requires its managed watcher/inbox and explicitly declared users.";
      }
    ];
  }
