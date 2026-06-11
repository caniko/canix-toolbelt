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
        options.services.rauthy = lib.mkOption {
          type = lib.types.submodule {
            freeformType = lib.types.attrsOf lib.types.anything;
            options = {
              configFile = lib.mkOption {
                type = lib.types.path;
                description = "Test stub for services.rauthy.configFile.";
              };
              provision = lib.mkOption {
                type = lib.types.submodule {
                  freeformType = lib.types.attrsOf lib.types.anything;
                };
                default = {};
                description = "Test stub for services.rauthy.provision.";
              };
            };
          };
          default = {};
          description = "Test stub for services.rauthy.";
        };
        config.services.rauthy.configFile = pkgs.writeText "rauthy-config.toml" "";
        options.networking.hosts = lib.mkOption {
          type = lib.types.attrsOf (lib.types.listOf lib.types.str);
          default = {};
          description = "Test stub for networking.hosts.";
        };
      })
      ../modules/nixos/services/rauthy-preset.nix
      {
        canix-toolbelt.services.rauthyPreset = {
          enable = true;
          package = pkgs.writeShellScriptBin "rauthy" "exit 0";
          hostname = "id.example.com";
          environmentFile = "/run/secrets/rauthy-env";
          adminEmail = "admin@example.com";
          webauthn = {
            rpId = "example.com";
            rpName = "Example";
          };
          mailLoopbackHostname = "mail.example.com";
          provision = {
            groups.internal = {};
            userAttributes.team.desc = "Team membership";
            scopes.team = {
              attrIncludeId = ["team"];
              claimsAtRoot = true;
            };
            providers.kanidm = {
              name = "Kanidm";
              issuer = "https://auth.example.com/oauth2/openid/rauthy";
            };
            clients.demo.redirectUris = ["https://demo.example.com/callback"];
            users."alice@example.com".groups = ["internal"];
          };
        };
      }
    ];
  };
  stateFileEval = lib.evalModules {
    specialArgs = {inherit pkgs;};
    modules = [
      ({lib, ...}: {
        options.services.rauthy = lib.mkOption {
          type = lib.types.submodule {
            freeformType = lib.types.attrsOf lib.types.anything;
            options.provision = lib.mkOption {
              type = lib.types.submodule {
                freeformType = lib.types.attrsOf lib.types.anything;
              };
              default = {};
              description = "Test stub for services.rauthy.provision.";
            };
          };
          default = {};
          description = "Test stub for services.rauthy.";
        };
        options.networking.hosts = lib.mkOption {
          type = lib.types.attrsOf (lib.types.listOf lib.types.str);
          default = {};
          description = "Test stub for networking.hosts.";
        };
      })
      ../modules/nixos/services/rauthy-preset.nix
      {
        canix-toolbelt.services.rauthyPreset = {
          enable = true;
          hostname = "id.example.com";
          environmentFile = "/run/secrets/rauthy-env";
          adminEmail = "admin@example.com";
          webauthn = {
            rpId = "example.com";
            rpName = "Example";
          };
          provision.stateFile = pkgs.writeText "rauthy-state.json" "{}";
        };
      }
    ];
  };
in
  pkgs.runCommand "rauthy-preset-eval" {} ''
    test ${lib.escapeShellArg eval.config.services.rauthy.settings.server.pub_url} = id.example.com
    test ${lib.escapeShellArg (toString eval.config.services.rauthy.settings.server.port_http)} = 8080
    test ${lib.escapeShellArg eval.config.services.rauthy.settings.webauthn.rp_origin} = https://id.example.com:443
    test ${lib.escapeShellArg eval.config.services.rauthy.provision.endpoint} = http://127.0.0.1:8080
    test ${lib.escapeShellArg (toString eval.config.services.rauthy.provision.generatedApiKey.enable)} = 1
    test ${lib.escapeShellArg (toString eval.config.services.rauthy.provision.transientApiKey.enable)} = 1
    test ${lib.escapeShellArg eval.config.services.rauthy.provision.generatedApiKey.environmentFile} = /run/secrets/rauthy-env
    test ${lib.escapeShellArg (builtins.head eval.config.networking.hosts."127.0.0.1")} = mail.example.com
    scopes=${lib.escapeShellArg (builtins.toJSON eval.config.services.rauthy.provision.scopes)}
    printf '%s' "$scopes" | grep -q 'claimsAtRoot'
    test -f ${stateFileEval.config.services.rauthy.provision.stateFile}
    state_file_keys=${lib.escapeShellArg (builtins.toJSON (builtins.attrNames stateFileEval.config.services.rauthy.provision))}
    printf '%s' "$state_file_keys" | grep -q 'stateFile'
    if printf '%s' "$state_file_keys" | grep -q 'groups'; then
      echo "stateFile mode must not forward declarative groups from the preset" >&2
      exit 1
    fi
    touch $out
  ''
