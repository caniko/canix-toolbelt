{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkEnableOption mkOption mkIf mkForce mkDefault types;
  inherit (lib.strings) hasPrefix concatStringsSep;

  cfg = config.canix-toolbelt.services.garage;

  isDefaultDir = path: hasPrefix "/var/lib/garage" path;

  dataDirPaths = let
    dd = cfg.settings.data_dir;
  in
    if builtins.isList dd
    then map (x: x.path) dd
    else [dd];

  allDataPaths = builtins.filter (p: !(isDefaultDir p)) (
    [cfg.settings.metadata_dir] ++ dataDirPaths
  );

  parentDirs = let
    parents = map (p: dirOf p) allDataPaths;
    outsideDefault = builtins.filter (p: !(isDefaultDir p)) parents;
  in
    lib.unique outsideDefault;

  dataDirToml = let
    dd = cfg.settings.data_dir;
  in
    if builtins.isList dd
    then
      concatStringsSep "\n" (
        map (x: ''
          [[data_dir]]
          path = ${builtins.toJSON x.path}
          capacity = ${builtins.toJSON x.capacity}
        '')
        dd
      )
    else "data_dir = ${builtins.toJSON dd}";

  optionalTomlTable = name: fields: let
    nonNull = builtins.filter (f: f.value != null) fields;
  in
    if nonNull == []
    then ""
    else
      "\n[${name}]\n"
      + concatStringsSep "\n" (
        map (f: "${f.key} = ${builtins.toJSON f.value}") nonNull
      )
      + "\n";

  optionalRpcPublicAddr = let
    v = cfg.settings.rpc_public_addr;
  in
    if v == null
    then ""
    else "\nrpc_public_addr = ${builtins.toJSON v}";

  optionalBootstrapPeers = let
    bs = cfg.settings.bootstrap_peers;
  in
    if bs == []
    then ""
    else "\nbootstrap_peers = ${builtins.toJSON bs}";

  adminTokenValue =
    if cfg.settings.admin.admin_token != null
    then cfg.settings.admin.admin_token
    else cfg.settings.rpc_secret;

  garageToml = pkgs.writeText "garage.toml" ''
    metadata_dir = ${builtins.toJSON cfg.settings.metadata_dir}
    ${dataDirToml}
    db_engine = ${builtins.toJSON cfg.settings.db_engine}
    block_size = ${toString cfg.settings.block_size}
    replication_factor = ${toString cfg.settings.replication_factor}
    rpc_secret = ${builtins.toJSON cfg.settings.rpc_secret}
    rpc_bind_addr = ${builtins.toJSON cfg.settings.rpc_bind_addr}${optionalRpcPublicAddr}${optionalBootstrapPeers}${optionalTomlTable "admin" [
      {
        key = "admin_token";
        value = adminTokenValue;
      }
      {
        key = "admin_bind_addr";
        value = cfg.settings.admin.admin_bind_addr;
      }
      {
        key = "metrics_token";
        value = cfg.settings.admin.metrics_token;
      }
    ]}${optionalTomlTable "s3_api" [
      {
        key = "api_bind_addr";
        value = cfg.settings.s3_api.api_bind_addr;
      }
      {
        key = "s3_region";
        value = cfg.settings.s3_api.s3_region;
      }
      {
        key = "api_root_domain";
        value = cfg.settings.s3_api.api_root_domain;
      }
    ]}${optionalTomlTable "s3_web" [
      {
        key = "bind_addr";
        value = cfg.settings.s3_web.bind_addr;
      }
      {
        key = "root_domain";
        value = cfg.settings.s3_web.root_domain;
      }
    ]}${optionalTomlTable "kubernetes" [
      {
        key = "api_bind_addr";
        value = cfg.settings.kubernetes.api_bind_addr;
      }
    ]}  '';
in {
  options.canix-toolbelt.services.garage = {
    enable = mkEnableOption "Garage Object Storage (S3 compatible)";

    package = mkOption {
      type = types.package;
      description = "Garage package to use. Must be set explicitly.";
    };

    logLevel = mkOption {
      type = types.enum ["error" "warn" "info" "debug" "trace"];
      default = "info";
      description = "Garage log level.";
    };

    extraEnvironment = mkOption {
      type = types.attrsOf types.str;
      default = {};
      description = "Extra environment variables to pass to the Garage server.";
    };

    environmentFile = mkOption {
      type = types.nullOr types.path;
      default = null;
      description = "File containing environment variables to be passed to the Garage server.";
    };

    settings = mkOption {
      type = types.submodule {
        freeformType = (pkgs.formats.toml {}).type;

        options = {
          metadata_dir = mkOption {
            type = types.str;
            default = "/var/lib/garage/meta";
            description = "The metadata directory, put on a fast disk (e.g. SSD) if possible.";
          };

          data_dir = mkOption {
            type = types.either types.str (
              types.listOf (
                types.submodule {
                  options = {
                    path = mkOption {
                      type = types.str;
                      description = "Path to the data directory.";
                    };
                    capacity = mkOption {
                      type = types.str;
                      description = "Capacity of this data directory (e.g. \"2T\").";
                    };
                  };
                }
              )
            );
            default = "/var/lib/garage/data";
            description = ''
              The directory in which Garage will store the data blocks of objects.
              Can be a single path string or a list of {path, capacity} attribute sets.
            '';
            example = [
              {
                path = "/var/lib/garage/data";
                capacity = "2T";
              }
            ];
          };

          db_engine = mkOption {
            type = types.enum ["lmdb" "sled" "sqlite"];
            default = "lmdb";
            description = "Database engine to use for metadata storage.";
          };

          block_size = mkOption {
            type = types.int;
            default = 1048576;
            description = "Block size for data storage in bytes.";
          };

          replication_factor = mkOption {
            type = types.int;
            default = 1;
            description = "Cluster replication factor. Must be at least 1.";
          };

          rpc_secret = mkOption {
            type = types.str;
            description = ''
              Shared secret for cluster-internal RPC authentication.
              Use a long, random string. Changing this on an existing cluster
              will break intra-cluster communication until all nodes are restarted.
            '';
          };

          rpc_bind_addr = mkOption {
            type = types.str;
            default = "[::]:3901";
            description = "Address and port for Garage RPC to bind.";
          };

          rpc_public_addr = mkOption {
            type = types.nullOr types.str;
            default = null;
            description = "Public address for Garage RPC cluster communications.";
          };

          bootstrap_peers = mkOption {
            type = types.listOf types.str;
            default = [];
            description = "List of bootstrap peer addresses for cluster discovery.";
            example = ["node1@example.com:3901"];
          };

          s3_api = mkOption {
            type = types.submodule {
              options = {
                api_bind_addr = mkOption {
                  type = types.str;
                  default = "[::]:3900";
                  description = "Address and port for the S3 API server.";
                };
                s3_region = mkOption {
                  type = types.str;
                  default = "garage";
                  description = "S3 region identifier.";
                };
                api_root_domain = mkOption {
                  type = types.nullOr types.str;
                  default = null;
                  description = "Root domain for S3 virtual-host style access.";
                  example = ".s3.example.com";
                };
              };
            };
            default = {};
            description = "S3 API configuration.";
          };

          s3_web = mkOption {
            type = types.submodule {
              options = {
                bind_addr = mkOption {
                  type = types.str;
                  default = "[::]:3902";
                  description = "Address and port for the S3 web server.";
                };
                root_domain = mkOption {
                  type = types.str;
                  default = ".web";
                  description = "Root domain for the S3 web server.";
                  example = ".web.example.com";
                };
              };
            };
            default = {};
            description = "S3 web configuration.";
          };

          admin = mkOption {
            type = types.submodule {
              options = {
                admin_token = mkOption {
                  type = types.nullOr types.str;
                  default = null;
                  description = ''
                    Token for admin API authentication. When null, garage falls
                    back to rpc_secret for admin authentication.
                  '';
                };
                admin_bind_addr = mkOption {
                  type = types.nullOr types.str;
                  default = null;
                  description = "Separate bind address for the admin API server.";
                };
                metrics_token = mkOption {
                  type = types.nullOr types.str;
                  default = null;
                  description = "Token for Prometheus metrics endpoint.";
                };
              };
            };
            default = {};
            description = "Admin API and metrics configuration.";
          };

          kubernetes = mkOption {
            type = types.submodule {
              options = {
                api_bind_addr = mkOption {
                  type = types.nullOr types.str;
                  default = null;
                  description = "Address and port for Kubernetes S3 API server.";
                };
              };
            };
            default = {};
            description = "Kubernetes S3 API configuration.";
          };
        };
      };
      default = {
        rpc_secret = lib.mkDefault "";
      };
      description = "Garage configuration. Settings are serialized to TOML with keys matching the garage config schema exactly.";
    };
  };

  config = mkIf cfg.enable {
    environment.etc."garage.toml".source = garageToml;

    environment.systemPackages = [
      (pkgs.writeScriptBin "garage" ''
        set -a
        [ -f ${lib.escapeShellArg cfg.environmentFile} ] && . ${lib.escapeShellArg cfg.environmentFile}
        exec ${lib.escapeShellArg (lib.getExe cfg.package)} "$@"
      '')
    ];

    # Static system user — avoids the DynamicUser + systemd-tmpfiles race
    # that prevents data directory creation on fresh deploys.
    users.users.garage = {
      isSystemUser = true;
      group = "garage";
    };
    users.groups.garage = {};

    systemd.services.garage = {
      description = "Garage Object Storage (S3 compatible)";
      after = ["network.target" "network-online.target"];
      wants = ["network.target" "network-online.target"];
      wantedBy = ["multi-user.target"];
      restartTriggers =
        [garageToml]
        ++ lib.optional (cfg.environmentFile != null) cfg.environmentFile;

      serviceConfig =
        {
          ExecStart = "${cfg.package}/bin/garage server";
          User = "garage";
          Group = "garage";
          DynamicUser = mkForce false;
          ProtectHome = true;
          NoNewPrivileges = true;
          LimitNOFILE = 42000;
          EnvironmentFile = lib.optional (cfg.environmentFile != null) cfg.environmentFile;
        }
        // lib.optionalAttrs (allDataPaths != []) {
          ReadWritePaths = allDataPaths;
        }
        // lib.optionalAttrs (allDataPaths == []) {
          StateDirectory = "garage";
        };

      environment =
        {
          RUST_LOG = mkDefault "garage=${cfg.logLevel}";
        }
        // cfg.extraEnvironment;
    };

    # Create data directories outside /var/lib/garage via tmpfiles.
    # Since we use a static user, the `garage` user exists at activation
    # time and systemd-tmpfiles-resetup can resolve it successfully.
    systemd.tmpfiles.rules =
      builtins.map (p: "d ${p} 0750 garage garage -") (parentDirs ++ allDataPaths);
  };
}
