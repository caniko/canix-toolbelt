{pkgs, ...}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  eval = lib.evalModules {
    specialArgs = {inherit pkgs;};
    modules = [
      ({
        lib,
        pkgs,
        ...
      }: {
        options.services.kanidm = lib.mkOption {
          type = lib.types.submodule {
            freeformType = lib.types.attrsOf lib.types.anything;
          };
          default = {};
          description = "Test stub for services.kanidm.";
        };
        options.services.kanidm-credentials = lib.mkOption {
          type = lib.types.submodule {
            freeformType = lib.types.attrsOf lib.types.anything;
          };
          default = {};
          description = "Test stub for services.kanidm-credentials.";
        };
        options.networking.hosts = lib.mkOption {
          type = lib.types.attrsOf (lib.types.listOf lib.types.str);
          default = {};
          description = "Test stub for networking.hosts.";
        };
        options.networking.firewall.allowedTCPPorts = lib.mkOption {
          type = lib.types.listOf lib.types.port;
          default = [];
          description = "Test stub for firewall ports.";
        };
        config.services.kanidm.package = pkgs.writeShellScriptBin "kanidmd" "exit 0";
      })
      ../modules/nixos/services/kanidm-preset.nix
      {
        canix-toolbelt.services.kanidmPreset = {
          enable = true;
          domain = "auth.example.com";
          tls = {
            chainFile = "/run/certs/auth/fullchain.pem";
            keyFile = "/run/certs/auth/key.pem";
          };
          loopbackHostname = "auth.example.com";
          provision = {
            adminPasswordFile = "/run/secrets/admin";
            idmAdminPasswordFile = "/run/secrets/idm-admin";
            extraJsonFile = pkgs.writeText "kanidm-extra.json" "{}";
          };
          credentials = {
            enable = true;
            package = pkgs.writeShellScriptBin "identity-cli" "exit 0";
            ldapUnixBind = true;
            posixAccounts.alice.passwordFile = "/run/secrets/alice-posix";
            serviceAccount = {
              name = "ldap-search";
              displayName = "LDAP search bind";
            };
          };
        };
      }
    ];
  };

  kanidm = eval.config.services.kanidm;
  inherit (kanidm) provision;
  credentials = eval.config.services."kanidm-credentials";
  hosts = eval.config.networking.hosts;
  firewallPorts = eval.config.networking.firewall.allowedTCPPorts;
in
  mkEvalCheck {
    name = "kanidm-preset-eval";
    resultMessage = "kanidm preset evaluated expected module defaults";
    assertions = [
      {
        name = "server-domain";
        assertion = kanidm.server.settings.domain == "auth.example.com";
        message = "expected Kanidm domain to match the preset domain";
      }
      {
        name = "server-bind-address";
        assertion = kanidm.server.settings.bindaddress == "127.0.0.1:8443";
        message = "expected Kanidm HTTPS bind address to use loopback and the default HTTPS port";
      }
      {
        name = "ldap-bind-address";
        assertion = kanidm.server.settings.ldapbindaddress == "[::]:3636";
        message = "expected Kanidm LDAP bind address to use the preset default";
      }
      {
        name = "tls-chain";
        assertion = kanidm.server.settings.tls_chain == "/run/certs/auth/fullchain.pem";
        message = "expected Kanidm TLS chain path to be forwarded";
      }
      {
        name = "provision-instance-url";
        assertion = provision.instanceUrl == "https://auth.example.com";
        message = "expected provision instanceUrl to default to the public domain";
      }
      {
        name = "provision-auto-remove";
        assertion = provision.autoRemove == false;
        message = "expected provision autoRemove to default to false";
      }
      {
        name = "credentials-instance-url";
        assertion = credentials.instanceUrl == "https://auth.example.com:8443";
        message = "expected credentials instanceUrl to include the HTTPS port";
      }
      {
        name = "credentials-ldap-url";
        assertion = credentials.ldapUrl == "ldaps://auth.example.com:3636";
        message = "expected credentials ldapUrl to use the LDAP default port";
      }
      {
        name = "posix-password-file";
        assertion = credentials.posixAccounts.alice.passwordFile == "/run/secrets/alice-posix";
        message = "expected POSIX account password file to be forwarded";
      }
      {
        name = "service-account-name";
        assertion = credentials.serviceAccount.name == "ldap-search";
        message = "expected service account name to be forwarded";
      }
      {
        name = "loopback-host";
        assertion = builtins.hasAttr "127.0.0.1" hosts && builtins.head hosts."127.0.0.1" == "auth.example.com";
        message = "expected loopback host entry for the public Kanidm domain";
      }
      {
        name = "ldap-firewall-port";
        assertion = builtins.elem 3636 firewallPorts;
        message = "expected LDAP firewall port 3636 to be opened";
      }
    ];
    runtimeScript = ''
      test -f ${lib.escapeShellArg (toString provision.extraJsonFile)}
    '';
  }
