{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.services.caddy;

  serverSubmodule = lib.types.submodule {
    options = {
      listen = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "Caddy listener addresses.";
      };
      routes = lib.mkOption {
        type = lib.types.listOf lib.types.attrs;
        default = [];
        description = "Rendered Caddy HTTP routes.";
      };
      blocks = lib.mkOption {
        type = lib.types.listOf lib.types.attrs;
        default = [];
        description = "Caddy error routes.";
      };
      cidrAllowlist = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "Source CIDRs allowed to use non-exempt hosts on this server.";
      };
      cidrExemptHosts = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "Hostnames exempt from this server's CIDR gate.";
      };
      metrics = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether to enable Caddy metrics for this server.";
      };
    };
  };
in {
  options.canix-toolbelt.services.caddy = {
    enable = lib.mkEnableOption "Caddy reverse proxy";

    servers = lib.mkOption {
      type = lib.types.attrsOf serverSubmodule;
      default = {};
      description = "Named Caddy HTTP servers.";
    };

    ingressListeners = lib.mkOption {
      type = lib.types.attrsOf (lib.types.listOf lib.types.str);
      default = {};
      description = "Listener addresses keyed by Fleetix ingress group name.";
    };

    cloudflareCidrs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "Cloudflare edge CIDRs allowed to reach cloudflare-access sites.";
    };

    staticRoots = lib.mkOption {
      type = lib.types.attrsOf lib.types.path;
      default = {};
      description = "Host-local static roots referenced by Fleetix files actions.";
    };

    tlsPolicies = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      default = [];
      description = "Caddy JSON TLS issuer policies.";
    };

    certificates = lib.mkOption {
      type = lib.types.listOf (lib.types.submodule {
        options = {
          certificate = lib.mkOption {
            type = lib.types.path;
            description = "PEM certificate file to load.";
          };
          key = lib.mkOption {
            type = lib.types.path;
            description = "PEM private key file to load.";
          };
        };
      });
      default = [];
      description = "PEM certificate/key pairs loaded through apps.tls.certificates.load_files.";
    };

    plugins = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "Caddy plugins as versioned Go module paths.";
    };

    pluginsHash = lib.mkOption {
      type = lib.types.str;
      default = "";
      description = "SRI hash for the combined Caddy plugin vendor directory.";
    };

    package = lib.mkOption {
      type = lib.types.package;
      default =
        if cfg.plugins != []
        then
          pkgs.caddy.withPlugins {
            inherit (cfg) plugins;
            hash = cfg.pluginsHash;
          }
        else pkgs.caddy;
      defaultText = lib.literalExpression ''
        if config.canix-toolbelt.services.caddy.plugins != []
        then pkgs.caddy.withPlugins {
          plugins = config.canix-toolbelt.services.caddy.plugins;
          hash = config.canix-toolbelt.services.caddy.pluginsHash;
        }
        else pkgs.caddy
      '';
      description = "Caddy package to run.";
    };

    authProviders = lib.mkOption {
      type = lib.types.attrsOf lib.types.attrs;
      default = {};
      description = "Caddy-security provider configs keyed by auth policy name.";
    };
  };

  config = lib.mkIf cfg.enable (let
    defaultLoggerName = "other";
    rollSizeMb = 25;

    getHostnameFromMatch = match: match.host or [];
    getHostnameFromRoute = route: lib.concatMap getHostnameFromMatch (route.match or []);
    allRoutes = lib.concatMap (server: server.routes) (builtins.attrValues cfg.servers);
    hostnames = lib.unique (lib.concatMap getHostnameFromRoute allRoutes);
    hostnameMap = builtins.listToAttrs (map (hostname: {
        name = builtins.head (lib.splitString "." hostname);
        value = hostname;
      })
      hostnames);

    routesWithGate = server:
      lib.optionals (server.cidrAllowlist != []) (
        lib.optional (server.cidrExemptHosts != []) {
          group = "cidr-allowlist";
          match = [{host = lib.unique server.cidrExemptHosts;}];
        }
        ++ [
          {
            group = "cidr-allowlist";
            match = [{not = [{remote_ip.ranges = server.cidrAllowlist;}];}];
            handle = [
              {
                handler = "static_response";
                status_code = "403";
              }
            ];
          }
        ]
      )
      ++ server.routes;

    serverConfig = _name: server: let
      serverHostnames = lib.unique (lib.concatMap getHostnameFromRoute server.routes);
      serverHostnameMap = builtins.listToAttrs (map (hostname: {
          name = builtins.head (lib.splitString "." hostname);
          value = hostname;
        })
        serverHostnames);
    in
      {
        inherit (server) listen;
        routes = routesWithGate server;
        errors.routes = server.blocks;
        logs = {
          default_logger_name = defaultLoggerName;
          logger_names =
            lib.mapAttrs' (name: value: {
              name = value;
              value = name;
            })
            serverHostnameMap;
        };
      }
      // lib.optionalAttrs server.metrics {metrics = {};};

    authHostnames = lib.unique (lib.concatMap (route: let
      hasAuth = builtins.any (handler: (handler.handler or "") == "authenticator") (route.handle or []);
    in
      lib.optionals hasAuth (lib.concatMap getHostnameFromMatch (route.match or [])))
    allRoutes);
  in {
    services.caddy = {
      enable = true;
      package = lib.mkDefault cfg.package;
      adapter = "''";
      configFile = pkgs.writeText "Caddyfile" (builtins.toJSON {
        apps =
          {
            http.servers = lib.mapAttrs serverConfig cfg.servers;
            tls =
              {
                automation.policies = cfg.tlsPolicies;
              }
              // lib.optionalAttrs (cfg.certificates != []) {
                certificates.load_files =
                  map (cert: {
                    inherit (cert) certificate key;
                  })
                  cfg.certificates;
              };
          }
          // lib.optionalAttrs (cfg.authProviders != {}) {
            security.config = {
              identity_providers =
                lib.mapAttrsToList (name: provider: {
                  inherit name;
                  kind =
                    if provider.driver or "generic" == "generic"
                    then "oauth"
                    else provider.driver;
                  params =
                    {
                      realm = name;
                      key_verification_disabled = true;
                      driver = provider.driver or "generic";
                    }
                    // lib.filterAttrs (n: _: n != "driver") provider;
                })
                cfg.authProviders;

              authentication_portals =
                lib.mapAttrsToList (name: _provider: {
                  inherit name;
                  identity_providers = [name];
                  cookie_config.domains = let
                    cookieDomain =
                      if authHostnames != []
                      then lib.concatStringsSep "." (lib.drop 1 (lib.splitString "." (builtins.head authHostnames)))
                      else "";
                  in
                    lib.optionalAttrs (cookieDomain != "") {"${cookieDomain}" = {};};
                  ui.private_links = [];
                })
                cfg.authProviders;

              authorization_policies =
                lib.mapAttrsToList (name: _provider: {
                  inherit name;
                  access_list_rules = [
                    {
                      conditions = ["match role authp/user"];
                      action = "allow";
                    }
                  ];
                  validate_bearer_header = true;
                  pass_claims_with_headers = true;
                })
                cfg.authProviders;
            };
          };

        logging.logs =
          {
            default = {
              level = "INFO";
              encoder.format = "console";
              writer.output = "stderr";
              exclude =
                (map (hostname: "http.log.access.${hostname}") (builtins.attrNames hostnameMap))
                ++ ["http.log.access.${defaultLoggerName}"];
            };
            other = {
              level = "INFO";
              encoder.format = "json";
              writer = {
                output = "file";
                filename = "${config.services.caddy.logDir}/other.log";
                mode = "0640";
                roll = true;
                roll_size_mb = rollSizeMb;
              };
              include = ["http.log.access.${defaultLoggerName}"];
            };
            admin = {
              level = "INFO";
              encoder.format = "json";
              writer = {
                output = "file";
                filename = "${config.services.caddy.logDir}/admin.log";
                mode = "0640";
                roll = true;
                roll_size_mb = rollSizeMb;
              };
              include = ["admin"];
            };
            tls = {
              level = "INFO";
              encoder.format = "json";
              writer = {
                output = "file";
                filename = "${config.services.caddy.logDir}/tls.log";
                mode = "0640";
                roll = true;
                roll_size_mb = rollSizeMb;
              };
              include = ["tls"];
            };
            debug = {
              level = "DEBUG";
              encoder.format = "json";
              writer = {
                output = "file";
                filename = "${config.services.caddy.logDir}/debug.log";
                mode = "0640";
                roll = true;
                roll_keep = 1;
                roll_size_mb = rollSizeMb;
              };
            };
          }
          // lib.mapAttrs (name: _value: {
            level = "INFO";
            encoder.format = "json";
            writer = {
              output = "file";
              filename = "${config.services.caddy.logDir}/${name}-access.log";
              mode = "0640";
              roll = true;
              roll_size_mb = rollSizeMb;
            };
            include = ["http.log.access.${name}"];
          })
          hostnameMap
          // lib.mapAttrs' (name: _value: {
            name = "${name}-error";
            value = {
              level = "ERROR";
              encoder.format = "json";
              writer = {
                output = "file";
                filename = "${config.services.caddy.logDir}/${name}-error.log";
                mode = "0640";
                roll = true;
                roll_size_mb = rollSizeMb;
              };
              include = ["http.log.access.${name}"];
            };
          })
          hostnameMap;
      });
    };

    systemd.services.caddy.serviceConfig = {
      AmbientCapabilities = "CAP_NET_BIND_SERVICE";
      LogRateLimitIntervalSec = "5s";
      LogRateLimitBurst = 100;
    };
  });
}
