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

  profilePath = "/nix/var/nix/profiles/system";
  currentSystemPath = "/run/current-system";

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
      backend = "nixos";
      inherit profilePath currentSystemPath;
      nixStore = "${config.nix.package}/bin/nix-store";
      nixEnv = "${config.nix.package}/bin/nix-env";
    };

  deploymentModule = {
    name,
    config,
    ...
  }:
    shared.mkDeploymentOptions {
      inherit name config pkgs;
      defaultStateRoot = "/var/lib/fleetix/deployment/${name}";
      defaultLockPath = "/run/lock/fleetix-deployment.lock";
      inherit policyFor;
    };

  timeoutFor = deployment:
    shared.timeoutStartSeconds deployment.commandTimeoutSeconds deployment.healthTimeoutSeconds;

  serviceFor = {
    deployment,
    description,
    arguments,
  }: {
    inherit description;
    # Root-owned activation needs the normal NixOS environment. Do not add
    # sandboxing that would block /etc, /run or profile writes.
    path = [config.nix.package pkgs.coreutils pkgs.systemd];
    serviceConfig = {
      Type = "oneshot";
      ExecStart = lib.escapeShellArgs arguments;
      TimeoutStartSec = timeoutFor deployment;
      TimeoutStopSec = 90;
      KillMode = "control-group";
      RestartForceExitStatus = [75];
      RestartSec = "30s";
    };
  };
in {
  options.services.recoverable-deployment.deployments = mkOption {
    type = types.attrsOf (types.submodule deploymentModule);
    default = {};
    description = ''
      Fixed-policy system deployments executed by the Fleetix deployment
      worker. Requests are untrusted; policy, verifier and health commands are
      root-owned configuration.
    '';
  };

  config = lib.mkMerge [
    {
      assertions = lib.concatLists (lib.mapAttrsToList (name: deployment: shared.assertions {inherit name deployment;}) active);
    }
    {
      systemd.services =
        lib.mapAttrs' (name: deployment: let
          runName = shared.runServiceName name;
          recoverName = shared.recoverServiceName name;
        in
          lib.nameValuePair runName (
            (serviceFor {
              inherit deployment;
              description = "Recoverable deployment ${deployment.target}";
              arguments = shared.runArguments {
                binary = shared.binary deployment.package;
                inherit (deployment) policyFile requestPath;
              };
            })
            // {
              onFailure = ["${recoverName}.service"];
              wantedBy = lib.optionals deployment.watchRequest ["multi-user.target"];
              unitConfig.ConditionPathExists = deployment.requestPath;
            }
          ))
        active
        // lib.mapAttrs' (name: deployment: let
          recoverName = shared.recoverServiceName name;
        in
          lib.nameValuePair recoverName ((serviceFor {
              inherit deployment;
              description = "Recover unsettled deployment ${deployment.target}";
              arguments = shared.recoverArguments {
                binary = shared.binary deployment.package;
                inherit (deployment) policyFile;
              };
            })
            // {
              wantedBy = ["multi-user.target"];
              after = ["local-fs.target"];
            }))
        active;

      systemd.timers = lib.mapAttrs' (name: deployment: let
        recoverName = shared.recoverServiceName name;
      in
        lib.nameValuePair recoverName {
          description = "Periodic recovery scan for deployment ${deployment.target}";
          wantedBy = ["timers.target"];
          timerConfig = {
            OnBootSec = "30s";
            OnUnitInactiveSec = "60s";
            AccuracySec = "1s";
            Unit = "${recoverName}.service";
          };
        })
      active;

      systemd.paths = lib.mapAttrs' (name: deployment: let
        runName = shared.runServiceName name;
      in
        lib.nameValuePair runName {
          description = "Watch deployment request for ${deployment.target}";
          wantedBy = ["paths.target"];
          pathConfig = {
            PathChanged = deployment.requestPath;
            Unit = "${runName}.service";
          };
        })
      watched;
    }
  ];
}
