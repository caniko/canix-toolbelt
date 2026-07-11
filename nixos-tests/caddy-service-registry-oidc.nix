{pkgs, ...}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  eval = lib.evalModules {
    specialArgs = {inherit pkgs;};
    modules = [
      ({lib, ...}: {
        options.age.secrets = lib.mkOption {
          type = lib.types.attrsOf (lib.types.submodule ({name, ...}: {
            freeformType = lib.types.attrsOf lib.types.anything;
            options.path = lib.mkOption {
              type = lib.types.str;
              default = "/run/agenix/${name}";
              description = "Test stub for agenix's generated runtime secret path.";
            };
          }));
          default = {};
          description = "Test stub for agenix secrets.";
        };

        options.assertions = lib.mkOption {
          type = lib.types.listOf lib.types.attrs;
          default = [];
          description = "Test stub for NixOS assertions.";
        };

        options.services.caddy = lib.mkOption {
          type = lib.types.submodule {
            freeformType = lib.types.attrsOf lib.types.anything;
          };
          default = {};
          description = "Test stub for services.caddy.";
        };

        options.systemd.services = lib.mkOption {
          type = lib.types.attrsOf (lib.types.submodule {
            freeformType = lib.types.attrsOf lib.types.anything;
          });
          default = {};
          description = "Test stub for systemd services.";
        };

        options.networking = lib.mkOption {
          type = lib.types.submodule {
            freeformType = lib.types.attrsOf lib.types.anything;
            options = {
              hostName = lib.mkOption {
                type = lib.types.str;
                default = "thething";
              };
              firewall = lib.mkOption {
                type = lib.types.submodule {
                  freeformType = lib.types.attrsOf lib.types.anything;
                };
                default = {};
              };
            };
          };
          default = {};
          description = "Test stub for networking.";
        };
      })
      ../modules/nixos/registry/hosts.nix
      ../modules/nixos/registry/services.nix
      ../modules/nixos/services/caddy-service-registry.nix
      {
        canix-toolbelt.hosts.thething.lanIp = "192.0.2.10";
        canix-toolbelt.services = {
          caddy = {
            enable = true;
            useServiceRegistry = true;
            oidc = {
              kanidmDomain = "auth.example.com";
              secretPath = name: ./fixtures + "/${name}_oidc_client_secret.age";
            };
          };

          reverseProxyServices = [
            {
              name = "searxng";
              hostname = "search.example.com";
              port = 8888;
              targetHost = "thething";
              auth.enable = true;
            }
          ];
        };
      }
    ];
  };

  cfg = eval.config.canix-toolbelt.services.caddy;
  secret = eval.config.age.secrets.searxng-oidc-client-secret;
  provider = cfg.authProviders.searxng;
  caddyService = eval.config.systemd.services.caddy;
  envService = eval.config.systemd.services.canix-caddy-oidc-env;
in
  mkEvalCheck {
    name = "caddy-service-registry-oidc";
    resultMessage = "caddy service registry OIDC generation evaluated expected defaults";
    assertions = [
      {
        name = "secret-name";
        assertion = secret.name == "searxng_oidc_client_secret";
        message = "expected generated agenix plaintext name to use underscores";
      }
      {
        name = "secret-generator";
        assertion = secret.generator.script == "alnum";
        message = "expected generated OIDC secret to use the alnum generator";
      }
      {
        name = "secret-owner-group-mode";
        assertion = secret.owner == "root" && secret.group == "kanidm" && secret.mode == "0440";
        message = "expected generated OIDC secret owner/group/mode defaults";
      }
      {
        name = "client-secret-file";
        assertion = cfg.oidc.clients.searxng.clientSecretFile == secret.path;
        message = "expected generated client metadata to expose the agenix runtime path";
      }
      {
        name = "provider-secret-env";
        assertion = provider.client_secret == "{env.SEARXNG_OIDC_CLIENT_SECRET}";
        message = "expected Caddy provider to reference the runtime secret environment variable";
      }
      {
        name = "provider-metadata-url";
        assertion = provider.metadata_url == "https://auth.example.com/oauth2/openid/searxng/.well-known/openid-configuration";
        message = "expected Kanidm metadata URL to be synthesized from the domain and service name";
      }
      {
        name = "caddy-env-file";
        assertion = caddyService.serviceConfig.EnvironmentFile == cfg.oidc.environmentFile;
        message = "expected caddy.service to read the generated OIDC environment file";
      }
      {
        name = "caddy-env-dependency";
        assertion =
          builtins.elem "canix-caddy-oidc-env.service" caddyService.requires
          && builtins.elem "canix-caddy-oidc-env.service" caddyService.after
          && builtins.elem "caddy.service" envService.before;
        message = "expected caddy.service to depend on the OIDC environment renderer";
      }
    ];
  }
