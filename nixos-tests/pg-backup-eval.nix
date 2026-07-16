{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  eval = import "${pkgs.path}/nixos/lib/eval-config.nix" {
    system = "x86_64-linux";
    modules = [
      ../modules/nixos/services/pg-backup.nix
      {
        system.stateVersion = "24.11";
        canix-toolbelt.services.pgBackup = {
          enable = true;
          role = "target";
          source.hostName = "10.0.0.2";
          targetSettings.replicatorPasswordFile = "/run/secrets/pg-replicator-password";
        };
      }
    ];
  };
  receive = eval.config.systemd.services.pg-receivewal.serviceConfig;
  base = eval.config.systemd.services.pg-basebackup.script;
in
  mkEvalCheck {
    name = "pg-backup-eval";
    resultMessage = "pg-backup creates slots before streaming and verifies base backups";
    assertions = [
      {
        name = "slot-create-is-pre-start";
        assertion = lib.hasInfix "--create-slot" (lib.concatStringsSep "\n" receive.ExecStartPre) && !(lib.hasInfix "--create-slot" receive.ExecStart);
        message = "pg_receivewal slot creation must be a one-shot ExecStartPre";
      }
      {
        name = "restart-policy";
        assertion = receive.Restart == "on-failure";
        message = "continuous WAL streaming must restart after failure";
      }
      {
        name = "verify-backup";
        assertion = lib.hasInfix "pg_verifybackup" base;
        message = "base backups must pass pg_verifybackup before publication";
      }
      {
        name = "partial-publish";
        assertion = lib.hasInfix ".partial" base && lib.hasInfix "mv \"$partial_dir\" \"$date_dir\"" base;
        message = "base backups must publish through a partial directory and atomic rename";
      }
    ];
  }
