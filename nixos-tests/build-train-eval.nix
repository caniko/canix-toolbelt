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
  retainedConfig = builtins.toFile "toolbelt-original-service.json" (builtins.readFile ../tests/fixtures/build-train-service.json);
  retainedContract = builtins.fromJSON (builtins.readFile retainedConfig);
  staged = evaluate {
    users.users.can.isNormalUser = true;
    canix-toolbelt.services.buildTrain = {
      builder = "atlas";
      user = lib.mkForce "can";
      package = lib.mkForce pkgs.coreutils;
      workers = 3;
      memoryMax = "2G";
      retainedDeployment = {
        package = pkgs.hello;
        user = "can";
        serviceConfig = retainedConfig;
      };
    };
  };
  stagedService = staged.systemd.services.fleetix-build-train.serviceConfig;
  stagedConnection = builtins.fromJSON staged.environment.etc."fleetix-train/connection.json".text;
  nextConnection = builtins.fromJSON staged.environment.etc."fleetix-train/next-connection.json".text;
in
  assert builtins.attrNames options.canix-toolbelt.services.buildTrain == builtins.attrNames options.fleetix.services.buildTrain;
  assert config.canix-toolbelt.services.buildTrain.workers == config.fleetix.services.buildTrain.workers;
  assert builtins.all (entry: !(lib.hasPrefix "buildTrain" entry.message) || entry.assertion) config.assertions;
  assert service.serviceConfig.User == "operator";
  assert service.serviceConfig.RuntimeDirectoryPreserve == "yes";
  assert service.serviceConfig.RuntimeDirectoryMode == "0700";
  assert service.serviceConfig.StateDirectoryMode == "0700";
  assert service.serviceConfig.UMask == "0077";
  assert service.serviceConfig.TimeoutStopSec == 21630;
  assert service.serviceConfig.MemoryMax == "1G";
  assert service.serviceConfig.KillMode == "control-group";
  assert !service.restartIfChanged && !service.stopIfChanged;
  assert service.serviceConfig.ExecStart == "${lib.getExe pkgs.hello} build-train serve --config ${config.environment.etc."fleetix-train/service.json".source}";
  assert connection.builder == "builder";
  assert connection.socket == "/run/fleetix-train/coordinator.sock";
  assert connection.gc_roots == "/nix/var/nix/gcroots/per-user/operator/fleetix-train";
  assert lib.elem "d ${connection.gc_roots} 0700 operator - -" config.systemd.tmpfiles.rules;
  assert stagedConnection.policy == retainedContract.coordinator.policy;
  assert stagedConnection.gc_roots == retainedContract.native.gc_roots;
  assert stagedService.ExecStart == "${lib.getExe pkgs.hello} build-train serve --config ${retainedConfig}";
  assert stagedService.MemoryMax == "1G";
  assert stagedService.User == "can";
  assert nextConnection.policy != stagedConnection.policy;
  assert nextConnection.gc_roots == stagedConnection.gc_roots;
  assert nextConnection.socket == stagedConnection.socket;
  assert staged.fleetix.services.buildTrain.workers == 3;
  assert builtins.all (entry: !(lib.hasPrefix "buildTrain" entry.message) || entry.assertion) staged.assertions;
  assert lib.elem pkgs.coreutils staged.system.extraDependencies;
  assert staged.environment.etc."fleetix-train/service.json".source == retainedConfig;
  assert builtins.any (entry: !entry.assertion && lib.hasInfix "two native queries" entry.message) invalid.assertions;
    pkgs.writeText "build-train-service-eval" "Private operator service, independent planning limits and retained activation policy verified\n"
