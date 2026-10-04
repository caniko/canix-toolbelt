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
in
  assert legacy.services.harbor-db.pgBackup.enable;
  assert legacy.services.harbor-db.pgBackup.sourceSettings.replicatorPasswordFile == "/run/secrets/replicator";
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
      echo "Legacy backup facade and explicit PostgreSQL storage guards compose" > $out
    ''
