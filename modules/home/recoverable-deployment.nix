{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkOption types;
  shared = import ../../lib/recoverableDeployment.nix {inherit lib;};
  cfg = config.services.recoverable-deployment;
  active = lib.filterAttrs (_: deployment: deployment.enable) cfg.deployments;
  watched = lib.filterAttrs (_: deployment: deployment.watchRequest) active;

  home = config.home.homeDirectory;
  stateParent = "${home}/.local/state/fleetix-deployment";
  profilePath = "${home}/.local/state/nix/profiles/home-manager";

  policyFor = deployment:
    shared.mkPolicy {
      inherit
        (deployment)
        target
        stateRoot
        lockPath
        admissionCommand
        healthCommand
        commandTimeoutSeconds
        healthTimeoutSeconds
        healthIntervalSeconds
        ;
      backend = "home-manager";
      inherit profilePath;
      currentSystemPath = profilePath;
      nixStore = "${pkgs.nix}/bin/nix-store";
      nixEnv = "${pkgs.nix}/bin/nix-env";
    };

  deploymentModule = {
    name,
    config,
    ...
  }:
    shared.mkDeploymentOptions {
      inherit name config pkgs;
      defaultStateRoot = "${stateParent}/${name}";
      defaultLockPath = "${stateParent}/deployment.lock";
      inherit policyFor;
    };

  timeoutFor = deployment:
    shared.timeoutStartSeconds deployment.commandTimeoutSeconds deployment.healthTimeoutSeconds;

  pathAssertion = label: path: {
    assertion = path != home && shared.underDirectory home path;
    message = "Recoverable deployment path '${label}' must stay inside the Home Manager user's home directory.";
  };

  homeAssertions = name: deployment:
    shared.assertions {inherit name deployment;}
    ++ [
      (pathAssertion "stateRoot" deployment.stateRoot)
      (pathAssertion "lockPath" deployment.lockPath)
      (pathAssertion "requestPath" deployment.requestPath)
      {
        assertion =
          shared.underDirectory stateParent deployment.lockPath
          || shared.underDirectory deployment.stateRoot deployment.lockPath;
        message = "Recoverable deployment '${name}' must keep its lock inside the private state parent or state root.";
      }
    ];

  serviceConfig = deployment: arguments: {
    Service = {
      Type = "oneshot";
      ExecStart = lib.escapeShellArgs arguments;
      TimeoutStartSec = timeoutFor deployment;
      TimeoutStopSec = 90;
      KillMode = "control-group";
      RestartForceExitStatus = [75];
      RestartSec = "30s";
      # Home Manager activation writes inside the user profile and state
      # directories. Keep the ordinary user environment instead of sandboxing
      # it away.
      Environment = ["PATH=${lib.makeBinPath [pkgs.nix pkgs.coreutils pkgs.systemd]}"];
    };
  };
in {
  options.services.recoverable-deployment.deployments = mkOption {
    type = types.attrsOf (types.submodule deploymentModule);
    default = {};
    description = ''
      Fixed-policy Home Manager deployments executed by the Fleetix deployment
      worker. Requests are untrusted; policy, verifier and health commands are
      user-owned configuration.
    '';
  };

  config = lib.mkMerge [
    {
      assertions = lib.concatLists (lib.mapAttrsToList (name: deployment: homeAssertions name deployment) active);
    }
    {
      systemd.user.services =
        lib.mapAttrs' (name: deployment: let
          runName = shared.runServiceName name;
          recoverName = shared.recoverServiceName name;
        in
          lib.nameValuePair runName ({
              Unit = {
                Description = "Recoverable deployment ${deployment.target}";
                OnFailure = ["${recoverName}.service"];
                ConditionPathExists = deployment.requestPath;
              };
              Install.WantedBy = lib.optionals deployment.watchRequest ["default.target"];
            }
            // serviceConfig deployment (shared.runArguments {
              binary = shared.binary deployment.package;
              inherit (deployment) policyFile requestPath;
            })))
        active
        // lib.mapAttrs' (name: deployment: let
          recoverName = shared.recoverServiceName name;
        in
          lib.nameValuePair recoverName ({
              Unit.Description = "Recover unsettled deployment ${deployment.target}";
              Install.WantedBy = ["default.target"];
            }
            // serviceConfig deployment (shared.recoverArguments {
              binary = shared.binary deployment.package;
              inherit (deployment) policyFile;
            })))
        active;

      systemd.user.timers = lib.mapAttrs' (name: deployment: let
        recoverName = shared.recoverServiceName name;
      in
        lib.nameValuePair recoverName {
          Unit.Description = "Periodic recovery scan for deployment ${deployment.target}";
          Timer = {
            OnBootSec = "30s";
            OnUnitInactiveSec = "60s";
            AccuracySec = "1s";
            Unit = "${recoverName}.service";
          };
          Install.WantedBy = ["timers.target"];
        })
      active;

      systemd.user.paths = lib.mapAttrs' (name: deployment: let
        runName = shared.runServiceName name;
      in
        lib.nameValuePair runName {
          Unit.Description = "Watch deployment request for ${deployment.target}";
          Path = {
            PathChanged = deployment.requestPath;
            Unit = "${runName}.service";
          };
          Install.WantedBy = ["paths.target"];
        })
      watched;
    }
  ];
}
