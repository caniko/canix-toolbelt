{pkgs}: let
  inherit (pkgs) lib;
  base = {
    options = {
      assertions = lib.mkOption {
        type = lib.types.listOf lib.types.attrs;
        default = [];
      };
      programs.opencode = {
        enable = lib.mkEnableOption "OpenCode";
        settings = lib.mkOption {
          inherit ((pkgs.formats.json {})) type;
          default = {};
        };
      };
      home.packages = lib.mkOption {
        type = lib.types.listOf lib.types.package;
        default = [];
      };
      home.sessionVariables = lib.mkOption {
        type = lib.types.attrsOf lib.types.str;
        default = {};
      };
      systemd.user.services = lib.mkOption {
        type = lib.types.attrs;
        default = {};
      };
    };
  };
  evaluate = enabled: extra:
    (lib.evalModules {
      specialArgs = {inherit pkgs;};
      modules = [
        base
        ../modules/home/headroom.nix
        {
          programs.opencode.enable = true;
          canix-toolbelt.opencodeJev = {
            enable = true;
            package = pkgs.emptyDirectory;
            credentialFile = "%t/agenix/typesafe";
            gateways.openai = {
              upstream = "https://api.openai.com/v1";
              port = 8790;
              paths = ["responses" "chat/completions"];
            };
          };
          canix-toolbelt.headroom = {
            enable = enabled;
            package = pkgs.emptyDirectory;
            opencodeGateways.openai = {
              url = "http://127.0.0.1:9700";
              unit = "infernix-headroom-openai.service";
            };
            claude = {
              enable = true;
              endpoint = {
                url = "http://127.0.0.1:9798";
                unit = "infernix-headroom-claude.service";
              };
            };
          };
        }
        extra
      ];
    }).config;
  enabled = evaluate true {};
  disabled = evaluate false {};
  derived = evaluate true ({config, ...}: {
    canix-toolbelt.headroom.opencodeGateways = lib.mkForce (lib.mapAttrs (name: gateway: {
        url = "http://127.0.0.1:${toString (gateway.port + 1000)}";
        unit = "infernix-headroom-${name}.service";
      })
      config.canix-toolbelt.opencodeJev.gateways);
  });
  invalid = evaluate true {canix-toolbelt.headroom.opencodeGateways.openai.url = lib.mkForce "https://external.invalid";};
  gateway = cfg: cfg.systemd.user.services.jev-gateway-openai;
in
  assert lib.all (a: a.assertion) enabled.assertions;
  assert lib.all (a: a.assertion) derived.assertions;
  assert builtins.elem "UPSTREAM_BASE_URL=http://127.0.0.1:9790/v1" (gateway derived).Service.Environment;
  assert !lib.all (a: a.assertion) invalid.assertions;
  assert enabled.canix-toolbelt.opencodeJev.gateways.openai.upstream == "https://api.openai.com/v1";
  assert (builtins.head enabled.programs.opencode.settings.plugins).options.routes."https://api.openai.com/v1/responses" == "http://127.0.0.1:8790/v1/responses";
  assert builtins.elem "UPSTREAM_BASE_URL=http://127.0.0.1:9700/v1" (gateway enabled).Service.Environment;
  assert (gateway enabled).Unit.Requires == ["infernix-headroom-openai.service"];
  assert enabled.home.sessionVariables.ANTHROPIC_BASE_URL == "http://127.0.0.1:9798";
  assert builtins.elem "UPSTREAM_BASE_URL=https://api.openai.com/v1" (gateway disabled).Service.Environment;
  assert disabled.home.sessionVariables == {}; true
