{
  pkgs,
  modules,
}: let
  evaluate = extra:
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      system = pkgs.stdenv.hostPlatform.system;
      modules = [modules.pg-backup modules.postgres-lifecycle extra];
    }).config;
  legacy = evaluate {
    services.postgresql = {
      enable = true;
      package = pkgs.postgresql_17;
    };
    canix-toolbelt.services.pgBackup = {
      enable = true;
      role = "source";
      source.hostName = "127.0.0.1";
      sourceSettings.replicatorPasswordFile = "/run/secrets/replicator";
    };
  };
  legacyTarget = evaluate {
    canix-toolbelt.services.pgBackup = {
      enable = true;
      role = "target";
      source.hostName = "127.0.0.1";
      targetSettings.replicatorPasswordFile = "/run/secrets/replicator";
    };
  };
  disabled = evaluate {};
  guarded = evaluate {
    services.postgresql = {
      enable = true;
      package = pkgs.postgresql_17;
    };
    services.harbor-db.postgresql = {
      enable = true;
      resource = "compat-cluster";
      switchAdoption.systemIdentifier = "12345";
    };
  };
  guard = guarded.system.preSwitchChecksScript;
  broken = evaluate {
    services.postgresql = {
      enable = true;
      package = pkgs.postgresql_17;
    };
    services.harbor-db.postgresql = {
      enable = true;
      switchAdoption.systemIdentifier = "12345";
    };
    system.preSwitchChecks.harbor-db-postgresql-adoption = pkgs.lib.mkForce (throw "injected broken adoption guard");
  };
  forcedBroken = builtins.tryEval (builtins.deepSeq broken.system.preSwitchChecks.harbor-db-postgresql-adoption true);
in
  # Demonstrate the previous existence-only predicate's false acceptance, then
  # require rejection when the injected generated value is actually forced.
  assert broken.system.preSwitchChecks ? harbor-db-postgresql-adoption;
  assert !forcedBroken.success;
  assert legacy.services.harbor-db.pgBackup.enable;
  assert legacy.services.harbor-db.pgBackup.sourceSettings.replicatorPasswordFile == "/run/secrets/replicator";
  assert legacyTarget.services.harbor-db.pgBackup.role == "target";
  assert legacyTarget.services.harbor-db.pgBackup.targetSettings.replicatorPasswordFile == "/run/secrets/replicator";
  assert pkgs.lib.hasInfix "--synchronous" legacyTarget.systemd.services.pg-receivewal.serviceConfig.ExecStart;
  assert !disabled.services.harbor-db.pgBackup.enable;
  assert !legacy.services.harbor-db.postgresql.enable;
  assert pkgs.lib.hasInfix "serve" guarded.systemd.services.postgresql.serviceConfig.ExecStart;
  assert guarded.services.postgresql.settings.fsync;
  assert guarded.services.postgresql.settings.full_page_writes;
  assert guarded.services.postgresql.settings.synchronous_commit == "on";
  assert guarded.system.preSwitchChecks ? harbor-db-postgresql-adoption;
  assert pkgs.lib.hasInfix "inspect-live" guarded.system.preSwitchChecks.harbor-db-postgresql-adoption;
  assert pkgs.lib.hasInfix "adopt-live" guarded.system.preSwitchChecks.harbor-db-postgresql-adoption;
  assert pkgs.lib.hasInfix "12345" guarded.system.preSwitchChecks.harbor-db-postgresql-adoption;
    pkgs.runCommand "harbor-db-compat-eval" {} ''
      echo 'RED: existence-only guard predicate accepts the injected fault; forced guard rejects it'
      # Realize the complete generated hook and its referenced lifecycle tool.
      test -f ${guard}
      ${pkgs.bash}/bin/bash -n ${guard}
      grep -F inspect-live ${guard}
      grep -F adopt-live ${guard}
      cp ${guard} "$out"
    ''
