{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.services.rauthyPreset;
  inherit (lib) mkEnableOption mkIf mkOption optionalAttrs types;
  format = pkgs.formats.toml {};

  attrs = types.attrsOf types.anything;
  pathOrString = types.oneOf [
    types.path
    types.str
  ];
  rauthyConfigFile = format.generate "rauthy-config.toml" config.services.rauthy.settings;
in {
  options.canix-toolbelt.services.rauthyPreset = {
    enable = mkEnableOption "reusable Rauthy server and provisioning defaults";

    package = mkOption {
      type = types.nullOr types.package;
      default = null;
      description = "Optional Rauthy package override.";
    };

    hostname = mkOption {
      type = types.str;
      description = "Public Rauthy hostname.";
    };

    httpPort = mkOption {
      type = types.port;
      default = 8080;
      description = "Local HTTP port Rauthy listens on.";
    };

    listenAddress = mkOption {
      type = types.str;
      default = "127.0.0.1";
      description = "Local address Rauthy listens on.";
    };

    environmentFile = mkOption {
      type = types.nullOr pathOrString;
      default = null;
      description = "Runtime environment file for Rauthy secrets.";
    };

    adminEmail = mkOption {
      type = types.str;
      description = "Bootstrap administrator email address.";
    };

    webauthn = {
      rpId = mkOption {
        type = types.str;
        description = "WebAuthn relying-party ID.";
      };

      rpName = mkOption {
        type = types.str;
        description = "WebAuthn relying-party display name.";
      };

      rpOrigin = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "WebAuthn relying-party origin. Defaults to https://<hostname>:443.";
      };
    };

    mailLoopbackHostname = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Optional mail hostname to pin to 127.0.0.1 for local SMTP hairpin avoidance.";
    };

    provision = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Whether to enable services.rauthy.provision.";
      };

      package = mkOption {
        type = types.nullOr types.package;
        default = null;
        description = "Optional rauthy-provision package override.";
      };

      groups = mkOption {
        type = attrs;
        default = {};
        description = "Rauthy groups passed to services.rauthy.provision.groups.";
      };

      stateFile = mkOption {
        type = types.nullOr pathOrString;
        default = null;
        description = "Optional pre-rendered rauthy-provision JSON state file.";
      };

      userAttributes = mkOption {
        type = attrs;
        default = {};
        description = "Rauthy custom user attributes.";
      };

      scopes = mkOption {
        type = attrs;
        default = {};
        description = "Rauthy custom scopes.";
      };

      providers = mkOption {
        type = attrs;
        default = {};
        description = "Upstream auth providers.";
      };

      clients = mkOption {
        type = attrs;
        default = {};
        description = "OIDC clients.";
      };

      users = mkOption {
        type = attrs;
        default = {};
        description = "Rauthy users.";
      };

      generatedApiKey = {
        enable = mkOption {
          type = types.bool;
          default = true;
          description = "Whether to use Rauthy's generated first-boot API-key flow.";
        };

        file = mkOption {
          type = types.str;
          default = "/var/lib/rauthy-provision/api-key";
          description = "Runtime path for the extracted generated API key.";
        };

        generatedSecretsFile = mkOption {
          type = types.str;
          default = "/var/lib/rauthy/bootstrap.secrets.enc";
          description = "Runtime path to Rauthy's encrypted generated bootstrap secret container.";
        };

        generatedSecretsTtl = mkOption {
          type = types.int;
          default = 0;
          description = "TTL in seconds for the generated bootstrap secret container. 0 disables automatic expiry.";
        };

        environmentFile = mkOption {
          type = types.nullOr pathOrString;
          default = null;
          description = "Environment file loaded by the generated API-key extraction unit.";
        };
      };

      transientApiKey = {
        enable = mkOption {
          type = types.bool;
          default = true;
          description = "Whether the provisioner mints a short-lived API key for each run.";
        };

        ttl = mkOption {
          type = types.ints.positive;
          default = 600;
          description = "Transient API-key lifetime in seconds.";
        };
      };
    };
  };

  config = mkIf cfg.enable {
    services.rauthy = let
      provision = mkIf cfg.provision.enable ({
          enable = true;
          endpoint = "http://${cfg.listenAddress}:${toString cfg.httpPort}";
        }
        // optionalAttrs (cfg.provision.package != null) {
          package = cfg.provision.package;
        }
        // (
          if cfg.provision.stateFile != null
          then {
            stateFile = cfg.provision.stateFile;
          }
          else {
            groups = cfg.provision.groups;
            userAttributes = cfg.provision.userAttributes;
            scopes = cfg.provision.scopes;
            providers = cfg.provision.providers;
            clients = cfg.provision.clients;
            users = cfg.provision.users;
          }
        )
        // {
          generatedApiKey = mkIf cfg.provision.generatedApiKey.enable {
            enable = true;
            configFile = rauthyConfigFile;
            environmentFile =
              if cfg.provision.generatedApiKey.environmentFile != null
              then cfg.provision.generatedApiKey.environmentFile
              else cfg.environmentFile;
            file = cfg.provision.generatedApiKey.file;
            generatedSecretsFile = cfg.provision.generatedApiKey.generatedSecretsFile;
            generatedSecretsTtl = cfg.provision.generatedApiKey.generatedSecretsTtl;
          };

          transientApiKey = mkIf cfg.provision.transientApiKey.enable {
            enable = true;
            ttl = cfg.provision.transientApiKey.ttl;
          };
        });
    in
      {
        inherit provision;

        enable = true;
        configurePostgres = true;

        settings = {
          server = {
            scheme = "http";
            listen_address = cfg.listenAddress;
            port_http = cfg.httpPort;
            pub_url = cfg.hostname;
            proxy_mode = true;
            trusted_proxies = ["127.0.0.1/32"];
          };

          bootstrap.admin_email = cfg.adminEmail;
          cluster.node_id = 1;

          webauthn = {
            rp_id = cfg.webauthn.rpId;
            rp_origin =
              if cfg.webauthn.rpOrigin != null
              then cfg.webauthn.rpOrigin
              else "https://${cfg.hostname}:443";
            rp_name = cfg.webauthn.rpName;
          };
        };
      }
      // optionalAttrs (cfg.package != null) {package = cfg.package;}
      // optionalAttrs (cfg.environmentFile != null) {environmentFile = cfg.environmentFile;};

    networking.hosts = mkIf (cfg.mailLoopbackHostname != null) {
      "127.0.0.1" = [cfg.mailLoopbackHostname];
    };
  };
}
