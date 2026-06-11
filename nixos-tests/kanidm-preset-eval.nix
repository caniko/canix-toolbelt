{pkgs, ...}: let
  inherit (pkgs) lib;

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
in
  pkgs.runCommand "kanidm-preset-eval" {} ''
    set -eux
    test ${lib.escapeShellArg eval.config.services.kanidm.server.settings.domain} = auth.example.com
    test ${lib.escapeShellArg eval.config.services.kanidm.server.settings.bindaddress} = 127.0.0.1:8443
    test ${lib.escapeShellArg eval.config.services.kanidm.server.settings.ldapbindaddress} = '[::]:3636'
    test ${lib.escapeShellArg eval.config.services.kanidm.server.settings.tls_chain} = /run/certs/auth/fullchain.pem
    test ${lib.escapeShellArg eval.config.services.kanidm.provision.instanceUrl} = https://auth.example.com
    test -z ${lib.escapeShellArg (toString eval.config.services.kanidm.provision.autoRemove)}
    test -f ${eval.config.services.kanidm.provision.extraJsonFile}
    test ${lib.escapeShellArg eval.config.services.kanidm-credentials.instanceUrl} = https://auth.example.com:8443
    test ${lib.escapeShellArg eval.config.services.kanidm-credentials.ldapUrl} = ldaps://auth.example.com:3636
    test ${lib.escapeShellArg eval.config.services.kanidm-credentials.posixAccounts.alice.passwordFile} = /run/secrets/alice-posix
    test ${lib.escapeShellArg eval.config.services.kanidm-credentials.serviceAccount.name} = ldap-search
    test ${lib.escapeShellArg (builtins.head eval.config.networking.hosts."127.0.0.1")} = auth.example.com
    ports=${lib.escapeShellArg (builtins.toJSON eval.config.networking.firewall.allowedTCPPorts)}
    printf '%s' "$ports" | grep -q 3636
    touch $out
  ''
