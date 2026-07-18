{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkEnableOption mkIf mkMerge mkOption types;
  cfg = config.canix-toolbelt.services.pgBackup;

  # A hostname is also used as a directory component. Keep the legacy IPv4
  # paths stable while preventing IPv6 / URI punctuation from becoming path
  # separators when the target list is expanded.
  sourceDirName = hostName: lib.replaceStrings ["/" ":" "[" "]"] ["_" "_" "_" "_"] hostName;

  mkTarget = name: source: let
    sourceId = source.hostName;
    sourceDir = sourceDirName sourceId;
    suffix =
      if name == "legacy"
      then ""
      else "-${name}";
    backupRoot = "${cfg.targetSettings.backupDir}/${sourceDir}";
    passwordFile =
      if cfg.targetSettings.replicatorPasswordFile != null
      then cfg.targetSettings.replicatorPasswordFile
      else "/run/invalid/pg-replicator-password";
    wrapBin = bin:
      pkgs.writers.writeBash "pg-backup-${name}-${baseNameOf bin}" ''
        set -euo pipefail
        password_file="${passwordFile}"
        if [ ! -r "$password_file" ] || [ ! -s "$password_file" ]; then
          echo "pg-backup: password file $password_file not readable or empty" >&2
          exit 1
        fi
        export PGPASSWORD="$(cat "$password_file")"
        exec ${cfg.targetSettings.package}/${bin} "$@"
      '';
    pgReceivewalCmd = "${wrapBin "bin/pg_receivewal"} -h ${sourceId} -p ${toString source.port} -U replicator";
    pgBasebackupCmd = "${wrapBin "bin/pg_basebackup"} -h ${sourceId} -p ${toString source.port} -U replicator";
    walDir = "${backupRoot}/wal";
    pruneBackupsScript = ''
      retain_days=${toString cfg.targetSettings.retain.baseBackupDays}
      wal_retain=${toString cfg.targetSettings.retain.walDays}

      prune_old_backups() {
        cutoff_epoch=$(date -d "$retain_days days ago" +%s)
        if [ -d "$backup_root/base" ]; then
          find "$backup_root/base" -maxdepth 1 -type d -name "????-??-??" | while read -r dir; do
            dir_date=$(basename "$dir")
            dir_epoch=$(date -d "$dir_date" +%s 2>/dev/null || true)
            if [ -n "$dir_epoch" ] && [ "$dir_epoch" -lt "$cutoff_epoch" ]; then
              echo "pg-backup: pruning old base backup $dir"
              rm -rf "$dir"
            fi
          done
        fi

        if [ -d "$wal_dir" ]; then
          find "$wal_dir" -maxdepth 2 -type f -name "????????????????????????????????" \
            -mtime +$wal_retain -delete 2>/dev/null || true
        fi
      }
    '';
  in {
    systemd.tmpfiles.rules = [
      "d ${backupRoot} 0750 postgres postgres -"
      "d ${backupRoot}/wal 0750 postgres postgres -"
      "d ${backupRoot}/base 0750 postgres postgres -"
    ];

    # `--create-slot` is a one-shot operation. It must be an ExecStartPre;
    # passing it to the long-running command makes pg_receivewal exit 0 after
    # creating the slot, which silently disables continuous WAL archiving.
    systemd.services."pg-receivewal${suffix}" = mkIf cfg.targetSettings.receiveWal.enable {
      description = "Receive WAL segments from ${sourceId}";
      unitConfig.RequiresMountsFor = backupRoot;
      after = ["network-online.target"];
      wants = ["network-online.target"];
      wantedBy = ["multi-user.target"];
      serviceConfig = {
        User = "postgres";
        ExecStartPre = ["${pgReceivewalCmd} -D ${walDir} --status-interval=5 --no-loop --slot=${cfg.targetSettings.receiveWal.slotName} --create-slot --if-not-exists"];
        ExecStart = "${pgReceivewalCmd} -D ${walDir} --verbose --slot=${cfg.targetSettings.receiveWal.slotName}";
        Restart = "on-failure";
        RestartSec = "5s";
        RestartSteps = 6;
        RestartMaxDelaySec = "5min";
        PrivateTmp = true;
        AmbientCapabilities = "";
        CapabilityBoundingSet = "";
        NoNewPrivileges = true;
        UMask = "0077";
      };
    };

    systemd.services."pg-backup-prune${suffix}" = {
      description = "Prune retained PostgreSQL backups from ${sourceId}";
      unitConfig.RequiresMountsFor = backupRoot;
      serviceConfig = {
        Type = "oneshot";
        User = "postgres";
        ProtectSystem = "strict";
        ReadWritePaths = [backupRoot];
        UMask = "0077";
      };
      script = ''
        set -euo pipefail
        backup_root="${backupRoot}"
        wal_dir="${walDir}"
        ${pruneBackupsScript}
        prune_old_backups
      '';
    };

    # Pull a base backup into a partial directory, verify its manifest, then
    # publish it atomically. A failed transfer therefore cannot masquerade as
    # a complete dated backup.
    systemd.services."pg-basebackup${suffix}" = mkIf cfg.targetSettings.baseBackup.enable {
      description = "Pull base backup from ${sourceId}";
      unitConfig.RequiresMountsFor = backupRoot;
      after = ["network-online.target" "pg-backup-prune${suffix}.service"];
      requires = ["pg-backup-prune${suffix}.service"];
      wants = ["network-online.target"];
      serviceConfig = {
        Type = "oneshot";
        User = "postgres";
        ProtectSystem = "strict";
        ReadWritePaths = [backupRoot];
        UMask = "0077";
      };
      script = ''
        set -euo pipefail

        backup_root="${backupRoot}"
        date_dir="$backup_root/base/$(date -I)"
        partial_dir="$date_dir.partial"
        wal_dir="${walDir}"
        ${pruneBackupsScript}

        rm -rf "$partial_dir"
        mkdir -p "$partial_dir"

        max_rate=${
          if cfg.targetSettings.baseBackup.maxRate != null
          then "'--max-rate=${cfg.targetSettings.baseBackup.maxRate}'"
          else "''"
        }
        slot=${cfg.targetSettings.baseBackup.slotName}

        # Ensure the temporary slot exists for the backup duration.
        ${pgReceivewalCmd} --status-interval=5 --no-loop --slot="$slot" --drop-slot 2>/dev/null || true
        ${pgReceivewalCmd} --status-interval=5 --no-loop --slot="$slot" --create-slot 2>/dev/null || true

        cleanup_slot() {
          ${pgReceivewalCmd} --slot="$slot" --drop-slot 2>/dev/null || true
        }
        trap cleanup_slot EXIT

        ${pgBasebackupCmd} -D "$partial_dir" --wal-method=stream \
          $max_rate --verbose --slot="$slot"

        ${cfg.targetSettings.package}/bin/pg_verifybackup "$partial_dir"
        rm -rf "$date_dir"
        mv "$partial_dir" "$date_dir"
        printf '%s\n' "$(date --iso-8601=seconds)" > "$backup_root/LAST_SUCCESS.tmp"
        chmod 0600 "$backup_root/LAST_SUCCESS.tmp"
        mv "$backup_root/LAST_SUCCESS.tmp" "$backup_root/LAST_SUCCESS"
        prune_old_backups
      '';
    };

    systemd.timers."pg-basebackup${suffix}" = mkIf cfg.targetSettings.baseBackup.enable {
      description = "Schedule daily PostgreSQL base backup from ${sourceId}";
      wantedBy = ["timers.target"];
      timerConfig = {
        OnCalendar = cfg.targetSettings.baseBackup.schedule;
        Persistent = true;
        RandomizedDelaySec = "30min";
      };
    };
  };
in {
  options.canix-toolbelt.services.pgBackup = {
    enable = mkEnableOption "PostgreSQL backup replication (source or target)";

    role = mkOption {
      type = types.enum ["source" "target"];
      description = ''
        Whether this host is the backup source (runs the PostgreSQL being
        backed up) or the backup target (receives WAL archives and pulls
        base backups).
      '';
    };

    source = {
      hostName = mkOption {
        type = types.str;
        example = "10.0.0.1";
        description = "Hostname or IP address of the source PostgreSQL server, reachable from the target.";
      };

      port = mkOption {
        type = types.port;
        default = 5432;
        description = "PostgreSQL port on the source server.";
      };
    };

    sourceSettings = {
      walLevel = mkOption {
        type = types.enum ["minimal" "replica" "logical"];
        default = "replica";
        description = "PostgreSQL wal_level. Must be replica or higher for replication.";
      };

      maxWalSenders = mkOption {
        type = types.ints.positive;
        default = 3;
        description = "Maximum concurrent WAL sender processes.";
      };

      maxReplicationSlots = mkOption {
        type = types.ints.positive;
        default = 2;
        description = "Maximum replication slots.";
      };

      listenAddresses = mkOption {
        type = types.listOf types.str;
        default = [];
        example = ["192.168.1.10"];
        description = "Exact IP addresses for PostgreSQL to listen on when replication is enabled.";
      };

      allowedReplicationHosts = mkOption {
        type = types.listOf types.str;
        default = [];
        example = ["192.168.1.20/32"];
        description = "CIDR-notation hosts allowed to connect for replication (added to pg_hba.conf).";
      };

      firewallInterface = mkOption {
        type = types.nullOr types.str;
        default = "wg-home";
        example = "eno1";
        description = "Interface on which to allow the PostgreSQL replication port, or null to manage the firewall elsewhere.";
      };

      replicatorPasswordFile = mkOption {
        type = types.nullOr types.path;
        default = null;
        description = "Path to a file containing the 'replicator' role password. Required on the source, optional on the target.";
      };
    };

    targetSettings = {
      package = mkOption {
        type = types.package;
        default = pkgs.postgresql;
        defaultText = lib.literalExpression "pkgs.postgresql";
        description = ''
          PostgreSQL package providing pg_receivewal, pg_basebackup, and
          pg_verifybackup. It should match the source server's major version.
        '';
      };

      backupDir = mkOption {
        type = types.path;
        default = "/var/backups/pgbackup";
        description = "Root backup storage directory. Each source gets its own WAL/base subdirectory.";
      };

      replicatorPasswordFile = mkOption {
        type = types.nullOr types.path;
        default = null;
        description = "Path to the replicator password file on the target host.";
      };

      receiveWal = {
        enable = mkEnableOption "continuous WAL streaming via pg_receivewal" // {default = true;};
        slotName = mkOption {
          type = types.str;
          default = "pgbackup_wal";
          description = "Replication slot name created on the source for pg_receivewal.";
        };
      };

      baseBackup = {
        enable = mkEnableOption "periodic base backup via pg_basebackup" // {default = true;};
        schedule = mkOption {
          type = types.str;
          default = "daily";
          description = "systemd OnCalendar schedule for base backup pulls.";
        };
        maxRate = mkOption {
          type = types.nullOr types.str;
          default = "20M";
          description = "Bandwidth limit for pg_basebackup (null = unlimited).";
        };
        slotName = mkOption {
          type = types.str;
          default = "pgbackup_base";
          description = "Temporary replication slot name created during base backup.";
        };
      };

      retain = {
        baseBackupDays = mkOption {
          type = types.ints.positive;
          default = 30;
          description = "Days to keep base backups.";
        };
        walDays = mkOption {
          type = types.ints.positive;
          default = 31;
          description = "Days to keep WAL segments; must be at least baseBackupDays + 1.";
        };
      };
    };
  };

  config = mkIf cfg.enable (let
    targetSources = {
      # Backwards-compatible singleton target. Existing deployments retain
      # the historical pg-* unit names and backup directory layout.
      legacy = {
        inherit (cfg.source) hostName port;
      };
    };
  in
    mkMerge [
      {
        assertions = [
          {
            assertion = cfg.role == "source" -> config.services.postgresql.enable or false;
            message = "canix-toolbelt.services.pgBackup (role=source) requires services.postgresql.enable = true on this host.";
          }
          {
            assertion = cfg.role != "source" || cfg.sourceSettings.replicatorPasswordFile != null;
            message = "canix-toolbelt.services.pgBackup (role=source) requires sourceSettings.replicatorPasswordFile to be set.";
          }
          {
            assertion = cfg.role != "target" || cfg.targetSettings.receiveWal.enable || cfg.targetSettings.baseBackup.enable;
            message = "canix-toolbelt.services.pgBackup (role=target) requires at least one target operation to be enabled.";
          }
          {
            assertion = cfg.role != "target" || cfg.targetSettings.retain.walDays >= cfg.targetSettings.retain.baseBackupDays + 1;
            message = "canix-toolbelt.services.pgBackup requires walDays >= baseBackupDays + 1 for safe PITR.";
          }
          {
            assertion = cfg.role != "target" || cfg.targetSettings.replicatorPasswordFile != null;
            message = "canix-toolbelt.services.pgBackup (role=target) requires targetSettings.replicatorPasswordFile to be set.";
          }
        ];
      }

      (mkIf (cfg.role == "source") {
        services.postgresql = {
          settings = mkMerge [
            (lib.optionalAttrs (cfg.sourceSettings.listenAddresses != []) {
              # NixOS' enableTCPIP setting otherwise forces '*'. Replication
              # should bind only to the explicitly selected interface addresses.
              listen_addresses = lib.mkForce (lib.concatStringsSep "," cfg.sourceSettings.listenAddresses);
            })
            {
              wal_level = lib.mkDefault cfg.sourceSettings.walLevel;
              max_wal_senders = lib.mkDefault cfg.sourceSettings.maxWalSenders;
              max_replication_slots = lib.mkDefault cfg.sourceSettings.maxReplicationSlots;
            }
          ];
          ensureUsers = [
            {
              name = "replicator";
              ensureClauses.replication = true;
            }
          ];
          authentication = lib.mkAfter (
            lib.concatMapStringsSep "\n" (host: "host replication replicator ${host} scram-sha-256")
            cfg.sourceSettings.allowedReplicationHosts
          );
        };

        systemd.services.postgresql-setup.script = lib.mkAfter (
          lib.optionalString (cfg.sourceSettings.replicatorPasswordFile != null) ''
            if [ ! -r ${cfg.sourceSettings.replicatorPasswordFile} ] || [ ! -s ${cfg.sourceSettings.replicatorPasswordFile} ]; then
              echo "pg-backup: replicator password file unreadable or empty" >&2
              exit 1
            fi
            printf '%s\n' \
              '\set replicator_password `cat ${cfg.sourceSettings.replicatorPasswordFile}`' \
              "ALTER ROLE replicator WITH PASSWORD :'replicator_password';" \
              | psql -d postgres
          ''
        );
      })

      (mkIf (cfg.role == "source" && cfg.sourceSettings.firewallInterface != null && cfg.sourceSettings.allowedReplicationHosts != []) {
        networking.firewall.interfaces."${cfg.sourceSettings.firewallInterface}".allowedTCPPorts = [cfg.source.port];
      })

      (mkIf (cfg.role == "target")
        (mkMerge [
          {
            # Preserve the legacy same-secret fallback for deployments that
            # put the agenix path under sourceSettings on both hosts.
            canix-toolbelt.services.pgBackup.targetSettings.replicatorPasswordFile = lib.mkDefault cfg.sourceSettings.replicatorPasswordFile;
          }
          (mkMerge (lib.mapAttrsToList mkTarget targetSources))
        ]))
    ]);
}
