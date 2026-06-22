{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkEnableOption mkIf mkMerge mkOption types;
  cfg = config.canix-toolbelt.services.pgBackup;
  sourceId = cfg.source.hostName;

  wrapBin = name: bin:
    pkgs.writers.writeBash name ''
      set -euo pipefail
      password_file="${cfg.targetSettings.replicatorPasswordFile}"
      if [ -r "$password_file" ]; then
        export PGPASSWORD="$(cat "$password_file")"
      else
        echo "pg-backup: password file $password_file not readable" >&2
        exit 1
      fi
      exec ${cfg.targetSettings.package}/${bin} "$@"
    '';

  pgReceivewalCmd = "${wrapBin "pg-receivewal" "bin/pg_receivewal"} -h ${sourceId} -p ${toString cfg.source.port} -U replicator";
  pgBasebackupCmd = "${wrapBin "pg-basebackup" "bin/pg_basebackup"} -h ${sourceId} -p ${toString cfg.source.port} -U replicator";
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
        description = "Extra IP addresses for PostgreSQL to listen on. The host's existing listen_addresses are preserved.";
      };

      allowedReplicationHosts = mkOption {
        type = types.listOf types.str;
        default = [];
        example = ["192.168.1.20/32"];
        description = "CIDR-notation hosts allowed to connect for replication (added to pg_hba.conf).";
      };

      replicatorPasswordFile = mkOption {
        type = types.nullOr types.path;
        default = null;
        description = "Path to a file containing the 'replicator' role password. Required on the source, optional on the target (use targetSettings.replicatorPasswordFile).";
      };
    };

    targetSettings = {
      package = mkOption {
        type = types.package;
        default = pkgs.postgresql;
        defaultText = lib.literalExpression "pkgs.postgresql";
        description = ''
          PostgreSQL package providing pg_receivewal and pg_basebackup.
          Should match the source server's major version for protocol
          compatibility (e.g. pkgs.postgresql_16 if the source runs PG 16).
        '';
      };

      backupDir = mkOption {
        type = types.path;
        default = "/var/backups/pgbackup";
        description = "Root backup storage directory. WALs go under <dir>/<source>/wal/, base backups under <dir>/<source>/base/.";
      };

      replicatorPasswordFile = mkOption {
        type = types.nullOr types.path;
        default = null;
        description = ''
          Path to the replicator password file on the target host.
          Uses sourceSettings.replicatorPasswordFile if this is null and
          role == "target" (useful when the same agenix secret is deployed
          to both hosts with the same path).
        '';
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
          description = "Days to keep base backups. Older ones are pruned after each fresh backup.";
        };

        walDays = mkOption {
          type = types.ints.positive;
          default = 31;
          description = "Days to keep WAL segments. Should be >= retain.baseBackupDays + 1 for safe PITR.";
        };
      };
    };
  };

  config = mkIf cfg.enable (mkMerge [
    # --- Assertions ---
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
          message = "canix-toolbelt.services.pgBackup (role=target) requires at least one of receiveWal or baseBackup to be enabled.";
        }
      ];
    }

    # --- Source-side: PostgreSQL replication tuning ---
    (mkIf (cfg.role == "source") {
      services.postgresql = {
        settings = mkMerge [
          (lib.optionalAttrs (cfg.sourceSettings.listenAddresses != []) {
            listen_addresses = lib.mkDefault (
              lib.concatStringsSep "," cfg.sourceSettings.listenAddresses
            );
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
            ensureClauses = {
              replication = true;
            };
          }
        ];

        authentication = lib.mkAfter (
          lib.concatMapStringsSep "\n" (host: ''
            host replication replicator ${host} scram-sha-256
          '')
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

      networking.firewall.interfaces."wg-home".allowedTCPPorts = lib.mkIf (cfg.sourceSettings.allowedReplicationHosts != []) [
        cfg.source.port
      ];
    })

    # --- Target-side: directories, WAL receiver, base backups ---
    (mkIf (cfg.role == "target") {
      canix-toolbelt.services.pgBackup.targetSettings.replicatorPasswordFile = lib.mkDefault cfg.sourceSettings.replicatorPasswordFile;

      systemd.tmpfiles.rules = [
        "d ${cfg.targetSettings.backupDir}/${sourceId}/wal 0750 postgres postgres -"
        "d ${cfg.targetSettings.backupDir}/${sourceId}/base 0750 postgres postgres -"
      ];

      # pg_receivewal: continuous WAL streaming from the source
      systemd.services.pg-receivewal = mkIf cfg.targetSettings.receiveWal.enable {
        description = "Receive WAL segments from ${sourceId}";
        after = ["network-online.target"];
        wants = ["network-online.target"];
        wantedBy = ["multi-user.target"];
        preStart = ''
          pwfile="${cfg.targetSettings.replicatorPasswordFile}"
          if [ -r "$pwfile" ]; then
            install -m 0600 "$pwfile" /tmp/.pgpass-receivewal
            # pgpass format: host:port:database:user:password
            line="${sourceId}:${toString cfg.source.port}:replication:replicator:$(cat "$pwfile")"
            echo "$line" > /tmp/.pgpass-receivewal
            chmod 0600 /tmp/.pgpass-receivewal
          else
            echo "pg-backup: password file $pwfile not found" >&2
            exit 1
          fi
        '';
        serviceConfig = {
          User = "postgres";
          ExecStart = "${pgReceivewalCmd} -D ${cfg.targetSettings.backupDir}/${sourceId}/wal --verbose --create-slot --if-not-exists --slot=${cfg.targetSettings.receiveWal.slotName}";
          Restart = "on-failure";
          RestartSec = "5s";
          PrivateTmp = true;
          AmbientCapabilities = "";
          CapabilityBoundingSet = "";
          NoNewPrivileges = true;
        };
      };

      # pg_basebackup: periodic full backup pull
      systemd.services.pg-basebackup = mkIf cfg.targetSettings.baseBackup.enable {
        description = "Pull base backup from ${sourceId}";
        after = ["network-online.target"];
        wants = ["network-online.target"];
        serviceConfig = {
          Type = "oneshot";
          User = "postgres";
        };
        script = ''
          set -euo pipefail

          backup_root="${cfg.targetSettings.backupDir}/${sourceId}"
          date_dir="$backup_root/base/$(date -I)"
          wal_dir="$backup_root/wal"

          # --- Pull base backup ---
          rm -rf "$date_dir"
          mkdir -p "$date_dir"

          max_rate=${
            if cfg.targetSettings.baseBackup.maxRate != null
            then "'--max-rate=${cfg.targetSettings.baseBackup.maxRate}'"
            else "''"
          }
          slot=${cfg.targetSettings.baseBackup.slotName}

          # Ensure the temporary slot exists for the backup duration
          ${pgReceivewalCmd} --status-interval=5 --no-loop --slot="$slot" --drop-slot 2>/dev/null || true
          ${pgReceivewalCmd} --status-interval=5 --no-loop --slot="$slot" --create-slot 2>/dev/null || true

          cleanup_slot() {
            ${pgReceivewalCmd} --slot="$slot" --drop-slot 2>/dev/null || true
          }
          trap cleanup_slot EXIT

          ${pgBasebackupCmd} -D "$date_dir" --wal-method=stream \
            $max_rate --verbose --slot="$slot"

          # --- Prune old base backups ---
          retain_days=${toString cfg.targetSettings.retain.baseBackupDays}
          find "$backup_root/base" -maxdepth 1 -type d -name "????-??-??" | while read -r dir; do
            dir_date=$(basename "$dir")
            if [ "$(date -d "$dir_date" +%s 2>/dev/null)" -lt "$(date -d "$retain_days days ago" +%s)" ]; then
              echo "pg-backup: pruning old base backup $dir"
              rm -rf "$dir"
            fi 2>/dev/null || true
          done

          # --- Prune old WALs ---
          wal_retain=${toString cfg.targetSettings.retain.walDays}
          find "$wal_dir" -maxdepth 2 -type f -name "????????????????????????????????????????" \
            -mtime +$wal_retain -delete 2>/dev/null || true
        '';
      };

      systemd.timers.pg-basebackup = mkIf cfg.targetSettings.baseBackup.enable {
        description = "Schedule daily PostgreSQL base backup from ${sourceId}";
        wantedBy = ["timers.target"];
        timerConfig = {
          OnCalendar = cfg.targetSettings.baseBackup.schedule;
          Persistent = true;
          RandomizedDelaySec = "30min";
        };
      };
    })
  ]);
}
