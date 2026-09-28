{pkgs}: let
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  projects = {
    media = {
      consumers = ["atlas"];
      runner = "atlas";
    };
    sdk.consumers = ["nomad"];
  };

  evaluate = registry: hostName:
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      system = "x86_64-linux";
      modules = [
        ../modules/nixos/services/attic-projects-registry.nix
        ({lib, ...}: {
          options.age.secrets = lib.mkOption {
            type = lib.types.attrsOf (lib.types.submodule {
              freeformType = lib.types.attrsOf lib.types.anything;
              options.path = lib.mkOption {
                type = lib.types.str;
                default = "/run/agenix/test-token";
              };
            });
            default = {};
          };
          config = {
            networking.hostName = hostName;
            canix-toolbelt.atticProjects = {
              inherit registry;
              secretPath = _: ./fixtures/attic-projects.json;
            };
          };
        })
      ];
    }).config;

  pathAtlas = evaluate ./fixtures/attic-projects.json "atlas";
  dataAtlas = evaluate projects "atlas";
  pathNomad = evaluate ./fixtures/attic-projects.json "nomad";
  dataNomad = evaluate projects "nomad";
in
  mkEvalCheck {
    name = "attic-projects-registry-eval";
    resultMessage = "Attic registry accepts resolved data and preserves JSON-path behavior";
    assertions = [
      {
        name = "atlas-tokens-match";
        assertion =
          builtins.attrNames pathAtlas.age.secrets
          == ["attic-media-token"]
          && pathAtlas.age.secrets == dataAtlas.age.secrets;
        message = "Atlas must get the same media token from either registry form";
      }
      {
        name = "atlas-runner-materialization-matches";
        assertion = pathAtlas.systemd.tmpfiles.settings."10-canix-attic-tokens" == dataAtlas.systemd.tmpfiles.settings."10-canix-attic-tokens";
        message = "the runner token directory and copy rules must be unchanged";
      }
      {
        name = "nomad-consumer-without-runner";
        assertion =
          builtins.attrNames pathNomad.age.secrets
          == ["attic-sdk-token"]
          && pathNomad.age.secrets == dataNomad.age.secrets
          && !(pathNomad.systemd.tmpfiles.settings ? "10-canix-attic-tokens")
          && !(dataNomad.systemd.tmpfiles.settings ? "10-canix-attic-tokens");
        message = "Nomad must consume only the SDK token without creating runner materialization";
      }
    ];
  }
