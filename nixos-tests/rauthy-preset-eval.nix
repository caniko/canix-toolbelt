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
        options.systemd.services = lib.mkOption {
          type = lib.types.attrsOf (lib.types.attrsOf lib.types.anything);
          default = {};
          description = "Test stub for systemd services.";
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
        options.systemd.services = lib.mkOption {
          type = lib.types.attrsOf (lib.types.attrsOf lib.types.anything);
          default = {};
          description = "Test stub for systemd services.";
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

  rauthy = eval.config.services.rauthy;
  provision = rauthy.provision;
  hosts = eval.config.networking.hosts;
  stateFileProvision = stateFileEval.config.services.rauthy.provision;
in
  mkEvalCheck {
    name = "rauthy-preset-eval";
    resultMessage = "rauthy preset evaluated expected module defaults";
    assertions = [
      {
        name = "public-url";
        assertion = rauthy.settings.server.pub_url == "id.example.com";
        message = "expected Rauthy public URL to match the preset hostname";
      }
      {
        name = "http-port";
        assertion = rauthy.settings.server.port_http == 8080;
        message = "expected Rauthy HTTP port to use the preset default";
      }
      {
        name = "webauthn-origin";
        assertion = rauthy.settings.webauthn.rp_origin == "https://id.example.com:443";
        message = "expected WebAuthn origin to default from the hostname";
      }
      {
        name = "provision-endpoint";
        assertion = provision.endpoint == "http://127.0.0.1:8080";
        message = "expected provision endpoint to use the loopback HTTP listener";
      }
      {
        name = "generated-api-key-enabled";
        assertion = provision.generatedApiKey.enable == true;
        message = "expected generated API-key provisioning to be enabled";
      }
      {
        name = "transient-api-key-enabled";
        assertion = provision.transientApiKey.enable == true;
        message = "expected transient API-key provisioning to be enabled";
      }
      {
        name = "generated-api-key-environment";
        assertion = provision.generatedApiKey.environmentFile == "/run/secrets/rauthy-env";
        message = "expected generated API-key unit to inherit the environment file";
      }
      {
        name = "mail-loopback-host";
        assertion = builtins.hasAttr "127.0.0.1" hosts && builtins.head hosts."127.0.0.1" == "mail.example.com";
        message = "expected mail loopback hostname to be registered";
      }
      {
        name = "scope-claims-at-root";
        assertion = provision.scopes.team.claimsAtRoot == true;
        message = "expected declarative scopes to be forwarded";
      }
      {
        name = "state-file-forwarded";
        assertion = builtins.hasAttr "stateFile" stateFileProvision;
        message = "expected stateFile mode to forward the rendered state file";
      }
      {
        name = "state-file-excludes-groups";
        assertion = !(builtins.hasAttr "groups" stateFileProvision);
        message = "expected stateFile mode not to forward declarative groups";
      }
    ];
    runtimeScript = ''
      test -f ${lib.escapeShellArg (toString stateFileProvision.stateFile)}
    '';
  }
