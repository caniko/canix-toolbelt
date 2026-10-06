{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.services.buildTrain;
  native = {
    nix = lib.getExe config.nix.package;
    timeout = "${pkgs.coreutils}/bin/timeout";
    timeout_seconds = cfg.workerTimeoutSeconds;
    query_timeout_seconds = cfg.queryTimeoutSeconds;
    system = pkgs.stdenv.hostPlatform.system;
    gc_roots = "/nix/var/nix/gcroots/per-user/${cfg.user}/fleetix-train";
    inherit (cfg) substitutes;
  };
  # Positional, versioned JSON: keep byte-for-byte parity with Rust's
  # policy_identity, independently of serde_json/preserve_order feature unification.
  identity = [
    "fleetix-train-policy"
    2
    cfg.builder
    cfg.admissionContract
    [native.nix native.timeout native.timeout_seconds native.query_timeout_seconds native.system native.gc_roots native.substitutes]
    [coordinator.socket coordinator.state_dir coordinator.workers coordinator.planning_workers coordinator.queue_limit coordinator.aging_seconds coordinator.planning_timeout_seconds]
    cfg.memoryMax
  ];
  policy = builtins.hashString "sha256" (builtins.toJSON identity);
  coordinator = {
    inherit policy;
    socket = "/run/fleetix-train/coordinator.sock";
    state_dir = "/var/lib/fleetix-train";
    inherit (cfg) workers;
    queue_limit = cfg.queueLimit;
    aging_seconds = cfg.agingSeconds;
    planning_workers = cfg.planningWorkers;
    planning_timeout_seconds = cfg.planningTimeoutSeconds;
  };
  serviceConfig = pkgs.writeText "fleetix-train-service.json" (builtins.toJSON {
    inherit native coordinator;
    inherit (cfg) builder;
    admission_contract = cfg.admissionContract;
    memory_max = cfg.memoryMax;
  });
  connection = {
    inherit policy;
    inherit (coordinator) socket;
    inherit (cfg) builder;
    preparation_dir = "/var/lib/fleetix-train/preparation";
    inherit (native) gc_roots;
  };
in {
  options.canix-toolbelt.services.buildTrain = {
    enable = lib.mkEnableOption "Fleetix's builder-local shared construction coordinator";
    package = lib.mkOption {
      type = lib.types.package;
      description = "Qualified canix-toolbelt package built with cli and build-train features.";
    };
    user = lib.mkOption {
      type = lib.types.str;
      description = "Existing authorized operator account; the private socket accepts this UID only.";
    };
    builder = lib.mkOption {
      type = lib.types.str;
      default = config.networking.hostName;
      description = "Canonical builder identity supplied by the consumer topology.";
    };
    admissionContract = lib.mkOption {
      type = lib.types.str;
      description = "Qualified host-resource admission policy identity. The train does not replace host admission.";
    };
    workers = lib.mkOption {
      type = lib.types.ints.between 1 64;
      default = 1;
    };
    queueLimit = lib.mkOption {
      type = lib.types.ints.positive;
      default = 128;
    };
    agingSeconds = lib.mkOption {
      type = lib.types.ints.positive;
      default = 300;
    };
    workerTimeoutSeconds = lib.mkOption {
      type = lib.types.ints.between 1 86400;
      default = 21600;
    };
    queryTimeoutSeconds = lib.mkOption {
      type = lib.types.ints.between 1 300;
      default = 60;
      description = "Per-command deadline for derivation planning and store queries.";
    };
    planningWorkers = lib.mkOption {
      type = lib.types.ints.between 1 16;
      default = 1;
      description = "Independent bounded graph planners; build workers remain available.";
    };
    planningTimeoutSeconds = lib.mkOption {
      type = lib.types.ints.between 1 86400;
      default = 180;
      description = "Graph preparation reply deadline, covering two query commands and kill grace periods.";
    };
    substitutes = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };
    memoryMax = lib.mkOption {
      type = lib.types.str;
      default = "1G";
      description = "Coordinator/CLI memory ceiling; Nix daemon builders retain their own host admission.";
    };
  };
  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = pkgs.stdenv.hostPlatform.isLinux;
        message = "buildTrain requires Linux peer authentication.";
      }
      {
        assertion = builtins.hasAttr cfg.user config.users.users;
        message = "buildTrain.user must be an existing operator account.";
      }
      {
        assertion = builtins.match "[a-z_][a-z0-9_-]*" cfg.user != null;
        message = "buildTrain.user must be a simple account name.";
      }
      {
        assertion = cfg.planningTimeoutSeconds >= 2 * cfg.queryTimeoutSeconds + 20;
        message = "buildTrain planning timeout must cover two native queries and their kill grace periods.";
      }
    ];
    environment.etc."fleetix-train/connection.json".text = builtins.toJSON connection;
    systemd.tmpfiles.rules = [
      "d /nix/var/nix/gcroots/per-user/${cfg.user} 0700 ${cfg.user} - -"
      "d ${native.gc_roots} 0700 ${cfg.user} - -"
      "d /var/lib/fleetix-train/preparation 0700 ${cfg.user} - -"
    ];
    systemd.services.fleetix-build-train = {
      description = "Fleetix dependency-aware shared construction";
      wantedBy = ["multi-user.target"];
      after = ["nix-daemon.service"];
      # An activating builder must drain its own train first. Keep the immutable
      # old coordinator policy alive until its requests finish; a changed client
      # policy is rejected rather than silently altering queued work.
      restartIfChanged = false;
      stopIfChanged = false;
      serviceConfig = {
        User = cfg.user;
        ExecStart = "${lib.getExe cfg.package} build-train serve --config ${serviceConfig}";
        RuntimeDirectory = "fleetix-train";
        RuntimeDirectoryMode = "0700";
        StateDirectory = "fleetix-train";
        StateDirectoryMode = "0700";
        UMask = "0077";
        Restart = "on-failure";
        RestartSec = 5;
        KillMode = "control-group";
        TimeoutStopSec = lib.max cfg.workerTimeoutSeconds cfg.planningTimeoutSeconds + 30;
        MemoryMax = cfg.memoryMax;
        NoNewPrivileges = true;
        PrivateTmp = true;
      };
    };
  };
}
