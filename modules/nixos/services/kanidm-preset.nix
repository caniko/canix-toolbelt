{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.services.kanidmPreset;
  inherit (lib) mkEnableOption mkIf mkOption optional optionalAttrs types;
  pathOrString = types.oneOf [
    types.path
    types.str
  ];
in {
  options.canix-toolbelt.services.kanidmPreset = {
    enable = mkEnableOption "reusable Kanidm server and provisioning defaults";

    package = mkOption {
      type = types.nullOr types.package;
      default = null;
      description = "Optional Kanidm package override.";
    };

    domain = mkOption {
      type = types.str;
      description = "Public Kanidm domain and OIDC issuer host.";
    };

    listenAddress = mkOption {
      type = types.str;
      default = "127.0.0.1";
      description = "Local HTTPS API address Kanidm listens on.";
    };

    httpsPort = mkOption {
      type = types.port;
      default = 8443;
      description = "Local HTTPS API port Kanidm listens on.";
    };

    ldapBindAddress = mkOption {
      type = types.str;
      default = "[::]:3636";
      description = "LDAP gateway bind address.";
    };

    openLdapFirewall = mkOption {
      type = types.bool;
      default = true;
      description = "Whether to open the LDAP TCP port in the firewall.";
    };

    tls = {
      chainFile = mkOption {
        type = pathOrString;
        description = "Runtime TLS fullchain path for Kanidm.";
      };

      keyFile = mkOption {
        type = pathOrString;
        description = "Runtime TLS key path for Kanidm.";
      };
    };

    loopbackHostname = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Optional hostname to pin to 127.0.0.1 for local self-provisioning hairpin avoidance.";
    };

    provision = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Whether to enable services.kanidm.provision.";
      };

      instanceUrl = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Provisioning URL. Defaults to https://<domain>.";
      };

      autoRemove = mkOption {
        type = types.bool;
        default = false;
        description = "Whether kanidm-provision should remove undeclared entities.";
      };

      adminPasswordFile = mkOption {
        type = types.nullOr pathOrString;
        default = null;
        description = "Runtime path containing the Kanidm admin password.";
      };

      idmAdminPasswordFile = mkOption {
        type = types.nullOr pathOrString;
        default = null;
        description = "Runtime path containing the Kanidm idm_admin password.";
      };

      extraJsonFile = mkOption {
        type = types.nullOr pathOrString;
        default = null;
        description = "Rendered kanidm-provision JSON to merge via services.kanidm.provision.extraJsonFile.";
      };
    };

    credentials = {
      enable = mkEnableOption "Kanidm credential reconciliation defaults";

      package = mkOption {
        type = types.nullOr types.package;
        default = null;
        description = "Optional identity-cli package override for services.kanidm-credentials.";
      };

      instanceUrl = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Kanidm API URL for credential reconciliation. Defaults to https://<domain>:<httpsPort>.";
      };

      ldapUrl = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Kanidm LDAP URL for token self-test. Defaults to ldaps://<domain>:3636.";
      };

      ldapUnixBind = mkOption {
        type = types.nullOr types.bool;
        default = null;
        description = "Optional domain-level ldap_allow_unix_pw_bind setting.";
      };

      posixAccounts = mkOption {
        type = types.attrsOf (types.submodule {
          options.passwordFile = mkOption {
            type = pathOrString;
            description = "Runtime path containing the POSIX/LDAP password.";
          };
        });
        default = {};
        description = "POSIX password accounts for services.kanidm-credentials.";
      };

      serviceAccount = mkOption {
        type = types.nullOr types.attrs;
        default = null;
        description = "Optional services.kanidm-credentials.serviceAccount attrset.";
      };
    };
  };

  config = mkIf cfg.enable {
    services.kanidm =
      {
        server = {
          enable = true;
          settings = {
            domain = cfg.domain;
            origin = "https://${cfg.domain}";
            bindaddress = "${cfg.listenAddress}:${toString cfg.httpsPort}";
            ldapbindaddress = cfg.ldapBindAddress;
            tls_chain = cfg.tls.chainFile;
            tls_key = cfg.tls.keyFile;
          };
        };

        provision = mkIf cfg.provision.enable {
          enable = true;
          instanceUrl =
            if cfg.provision.instanceUrl != null
            then cfg.provision.instanceUrl
            else "https://${cfg.domain}";
          autoRemove = cfg.provision.autoRemove;
          adminPasswordFile = cfg.provision.adminPasswordFile;
          idmAdminPasswordFile = cfg.provision.idmAdminPasswordFile;
          extraJsonFile = cfg.provision.extraJsonFile;
        };
      }
      // optionalAttrs (cfg.package != null) {package = cfg.package;};

    services.kanidm-credentials = mkIf cfg.credentials.enable ({
        enable = true;
        instanceUrl =
          if cfg.credentials.instanceUrl != null
          then cfg.credentials.instanceUrl
          else "https://${cfg.domain}:${toString cfg.httpsPort}";
        ldapUrl =
          if cfg.credentials.ldapUrl != null
          then cfg.credentials.ldapUrl
          else "ldaps://${cfg.domain}:3636";
        idmAdminPasswordFile = cfg.provision.idmAdminPasswordFile;
        adminPasswordFile = cfg.provision.adminPasswordFile;
        ldapUnixBind = cfg.credentials.ldapUnixBind;
        posixAccounts = cfg.credentials.posixAccounts;
        serviceAccount = cfg.credentials.serviceAccount;
      }
      // optionalAttrs (cfg.credentials.package != null) {package = cfg.credentials.package;});

    networking.hosts = mkIf (cfg.loopbackHostname != null) {
      "127.0.0.1" = [cfg.loopbackHostname];
    };

    networking.firewall.allowedTCPPorts = optional cfg.openLdapFirewall 3636;
  };
}
