{
  pkgs,
  inputs,
}: let
  lib = pkgs.lib;
  fixture = adjacent: let
    evaluated = import "${pkgs.path}/nixos/lib/eval-config.nix" {
      system = pkgs.stdenv.hostPlatform.system;
      modules = [
        inputs.home-manager.nixosModules.home-manager
        ../modules/nixos/services/executor-admission.nix
        {
          system.stateVersion = "24.11";
          canix-toolbelt.profiles.near-builder = {
            default = true;
            enable = lib.mkForce adjacent;
            specialisations.away.enable = false;
          };
          canix-toolbelt.services.executorAdmission.secondary = {
            enable = true;
            profile = "near-builder";
          };
          canix-toolbelt.activation.contracts.secondary = {
            enabled = true;
            owner = "fixture";
            rollout = "declarative";
            units = ["executor.service"];
          };
          systemd.services.executor = {
            wantedBy = ["multi-user.target"];
            restartTriggers = ["unchanged-worker-config"];
            serviceConfig = {
              ExecStart = "${pkgs.coreutils}/bin/sleep infinity";
              User = "nobody";
              MemoryMax = "1G";
            };
          };
        }
      ];
    };
  in
    evaluated.config;
  adjacent = fixture true;
  detached = fixture false;
  policy = cfg: builtins.fromJSON cfg.environment.etc."executor-secondary-admission.json".text;
  worker = cfg: cfg.systemd.services.executor;
  path = adjacent.canix-toolbelt.services.executorAdmission.secondary.file;
in
  assert (policy adjacent) == {
    version = 1;
    accepting = true;
  };
  assert (policy detached) == {
    version = 1;
    accepting = false;
  };
  assert adjacent.environment.etc."executor-secondary-admission.json".mode == "0444";
  assert (worker adjacent).serviceConfig == (worker detached).serviceConfig;
  assert (worker adjacent).restartTriggers == (worker detached).restartTriggers;
  assert (worker adjacent).requires == (worker detached).requires;
  assert (worker adjacent).wantedBy == (worker detached).wantedBy;
  assert !(builtins.elem path (worker adjacent).restartTriggers);
  assert lib.any (artifact: artifact.path == path && !artifact.sensitive)
  adjacent.canix-toolbelt.activation.contracts.secondary.artifacts;
  assert !(lib.any (entry: lib.hasPrefix "Executor admission" entry.message && !entry.assertion)
    (adjacent.assertions ++ detached.assertions));
    pkgs.writeText "executor-admission-eval" "Profile detachment changes only admission; executor service and ownership remain stable.\n"
