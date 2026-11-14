{
  pkgs,
  inputs,
}: let
  inherit (pkgs) lib;
  evaluate = overrides:
    (evaluateModule overrides).config;
  evaluateModule = overrides:
    import "${pkgs.path}/nixos/lib/eval-config.nix" {
      system = pkgs.stdenv.hostPlatform.system;
      modules = [
        (import ../modules/nixos/services/build-train.nix {
          fleetixBuildTrain = inputs.fleetix.nixosModules.build-train;
        })
        {
          networking.hostName = "builder";
          users.users.operator = {isNormalUser = true;};
          canix-toolbelt.services.buildTrain = {
            enable = true;
            user = "operator";
            package = pkgs.hello;
            admissionContract = "fixture-resource-admission";
          };
        }
        overrides
      ];
    };
  config = evaluate {};
  service = config.systemd.services.fleetix-build-train;
  connection = builtins.fromJSON config.environment.etc."fleetix-train/connection.json".text;
  invalid = evaluate {
    canix-toolbelt.services.buildTrain.planningTimeoutSeconds = 30;
  };
  inherit ((evaluateModule {})) options;
  user = evaluate {canix-toolbelt.services.buildTrain.serviceScope = "user";};
in
  assert builtins.attrNames options.canix-toolbelt.services.buildTrain == builtins.attrNames options.fleetix.services.buildTrain;
  assert config.canix-toolbelt.services.buildTrain.workers == config.fleetix.services.buildTrain.workers;
  # Successful NixOS assertions may have failure-only diagnostic expressions.
  assert builtins.all (entry: entry.assertion || !(lib.hasPrefix "buildTrain" entry.message)) config.assertions;
  assert service.serviceConfig.User == "operator";
  assert service.serviceConfig.RuntimeDirectoryPreserve == "yes";
  assert service.serviceConfig.RuntimeDirectoryMode == "0700";
  assert service.serviceConfig.StateDirectoryMode == "0700";
  assert service.serviceConfig.UMask == "0077";
  assert service.serviceConfig.TimeoutStopSec == 21630;
  assert service.serviceConfig.MemoryMax == "1G";
  assert service.serviceConfig.KillMode == "control-group";
  assert service.restartIfChanged;
  assert service.serviceConfig.ExecStart == "${lib.getExe pkgs.hello} build-train serve --config ${config.environment.etc."fleetix-train/service.json".source}";
  assert connection.builder == "builder";
  assert connection.socket == "/run/fleetix-train/coordinator.sock";
  assert connection.gc_roots == "/nix/var/nix/gcroots/per-user/operator/fleetix-train";
  assert lib.elem "d ${connection.gc_roots} 0700 operator - -" config.systemd.tmpfiles.rules;
  assert builtins.any (entry: !entry.assertion && lib.hasInfix "two native queries" entry.message) invalid.assertions;
  assert config.canix-toolbelt.services.buildTrain.serviceScope == "system";
  assert user.fleetix.services.buildTrain.serviceScope == "user";
  assert !(user.systemd.services ? fleetix-build-train);
  assert user.systemd.user.services.fleetix-build-train.unitConfig.ConditionUser == "operator";
    pkgs.writeText "build-train-service-eval" "Private operator service, independent planning limits and restart-on-change policy verified\n"
