{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.services.caddy;
in {
  options.canix-toolbelt.services.caddy = {
    enable = lib.mkEnableOption "Caddy reverse proxy";

    tlsPolicies = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      default = [];
      description = "Caddy JSON TLS issuer policies.";
    };

    routes = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      default = [];
      description = "Caddy JSON routes for HTTP servers.";
    };

    blocks = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      default = [];
      description = "Caddy JSON error blocks for HTTP servers.";
    };

    cidrAllowlist = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "CIDR blocks to allow for requests.";
    };

    cidrExemptHosts = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "Hostnames exempt from the CIDR allowlist.";
    };

    goatcounterUrl = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "GoatCounter base URL for callers that inject analytics into routes.";
    };

    plugins = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "Caddy plugins, as Go module paths with versions, to include in the build.";
    };

    pluginsHash = lib.mkOption {
      type = lib.types.str;
      default = "";
      description = "SRI hash for the combined Caddy plugins vendor directory.";
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
      description = ''
        OIDC provider configs keyed by portal name. Each value is an
        attrs passed to caddy-security's oauth2_providers entry (driver,
        client_id, metadata_url, scopes, etc.). Consumed by
        mkAuthServiceRoute in the service registry.
      '';
    };
  };

  config = lib.mkIf cfg.enable (let
    defaultLoggerName = "other";
    rollSizeMb = 25;

    getHostnameFromMatch = match:
      if lib.hasAttr "host" match
      then match.host
      else [];
    getHostnameFromRoute = route:
      if lib.hasAttr "match" route
      then lib.concatMap getHostnameFromMatch route.match
      else [];
    hostnames = lib.unique (lib.concatMap getHostnameFromRoute cfg.routes);
    hostnameMap = builtins.listToAttrs (
      map (hostname: {
        name = builtins.head (lib.splitString "." hostname);
        value = hostname;
      })
      hostnames
    );
  in {
    canix-toolbelt.services.caddy = {
      cidrAllowlist = ["127.0.0.1/32"];

      routes = lib.mkBefore (
        lib.optional (cfg.cidrExemptHosts != []) {
          group = "cidr-allowlist";
          match = [{host = lib.unique cfg.cidrExemptHosts;}];
        }
        ++ [
          {
            group = "cidr-allowlist";
            match = [{not = [{remote_ip.ranges = cfg.cidrAllowlist;}];}];
            handle = [
              {
                handler = "static_response";
                status_code = "403";
              }
            ];
          }
        ]
      );
    };

    services.caddy = {
      enable = true;
      package = lib.mkDefault cfg.package;
      adapter = "''";
      configFile = pkgs.writeText "Caddyfile" (
        builtins.toJSON ({
          apps = {
            http.servers.main = {
              listen = [":443"];

              inherit (cfg) routes;
              errors.routes = cfg.blocks;

              logs = {
                default_logger_name = defaultLoggerName;
                logger_names =
                  lib.mapAttrs' (name: value: {
                    name = value;
                    value = name;
                  })
                  hostnameMap;
              };

              metrics = {};
            };

            tls.automation.policies = cfg.tlsPolicies;
          }
          // lib.optionalAttrs (cfg.authProviders != {}) {
            security.config = {
              identity_providers = lib.mapAttrsToList (name: provider: {
                inherit name;
                kind = if provider.driver or "generic" == "generic" then "oauth" else provider.driver;
                params = {realm = name; key_verification_disabled = true; driver = provider.driver or "generic";} // lib.filterAttrs (n: _: n != "driver") provider;
              }) cfg.authProviders;

              authentication_portals = lib.mapAttrsToList (name: _provider: {
                inherit name;
                identity_providers = [name];
                cookie_config.domains =
                  let
                    authHostnames = lib.unique (lib.concatMap (route:
                      let
                        hasAuth = builtins.any (h: let hn = h.handler or ""; in hn == "authenticator" || hn == "http.handlers.authenticator") (route.handle or []);
                      in
                        lib.optionals hasAuth (lib.concatMap (m: m.host or []) (route.match or []))
                    ) cfg.routes);
                    cookieDomain =
                      if authHostnames != []
                      then lib.concatStringsSep "." (lib.drop 1 (lib.splitString "." (builtins.head authHostnames)))
                      else "";
                  in
                    if cookieDomain != "" then {"${cookieDomain}" = {};} else {};
                ui.private_links = [];
              }) cfg.authProviders;

              authorization_policies = lib.mapAttrsToList (name: _provider: {
                inherit name;
                access_list_rules = [{
                  conditions = ["match role authp/user"];
                  action = "allow";
                }];
                validate_bearer_header = true;
                pass_claims_with_headers = true;
              }) cfg.authProviders;
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
                  ++ [
                    "http.log.access.${defaultLoggerName}"
                  ];
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
          // (lib.mapAttrs (name: _value: {
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
            hostnameMap)
          // (lib.mapAttrs' (name: _value: {
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
            hostnameMap);
        })
      );
    };

    systemd.services.caddy.serviceConfig = {
      AmbientCapabilities = "CAP_NET_BIND_SERVICE";
      LogRateLimitIntervalSec = "5s";
      LogRateLimitBurst = 100;
    };

    networking.firewall = {
      allowedTCPPorts = [
        80
        443
      ];
      allowedUDPPorts = [443];
    };
  });
}
