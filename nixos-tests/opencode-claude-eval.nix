{
  pkgs,
  fleetixLib,
}: let
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
      home.homeDirectory = lib.mkOption {
        type = lib.types.str;
        default = "/home/example";
      };
      xdg.configHome = lib.mkOption {
        type = lib.types.str;
        default = "/home/example/.config";
      };
      systemd.user.services = lib.mkOption {
        type = lib.types.attrs;
        default = {};
      };
    };
  };
  topology.services.endpoints = {
    claude = {
      targetHost = "example";
      port = 3460;
      bind = "loopback";
      transport = "http";
    };
    jev = {
      targetHost = "example";
      port = 8792;
      bind = "loopback";
      transport = "http";
    };
  };
  evaluate = extra:
    (lib.evalModules {
      specialArgs = {inherit pkgs;};
      modules = [base (import ../modules/home/opencode-claude.nix {inherit fleetixLib;}) extra];
    }).config;
  enabled = evaluate {
    programs.opencode.enable = true;
    canix-toolbelt.opencodeJev = {
      enable = true;
      package = pkgs.emptyDirectory;
      credentialFile = "%t/agenix/typesafe";
    };
    canix-toolbelt.opencodeClaude = {
      enable = true;
      package = pkgs.emptyDirectory;
      inherit topology;
      hostName = "example";
      meridianEndpoint = "claude";
      jevEndpoint = "jev";
    };
  };
  invalid = evaluate {
    programs.opencode.enable = true;
    canix-toolbelt.opencodeJev = {
      enable = true;
      package = pkgs.emptyDirectory;
      credentialFile = "/runtime/key";
    };
    canix-toolbelt.opencodeClaude = {
      enable = true;
      package = pkgs.emptyDirectory;
      topology.services.endpoints = topology.services.endpoints // {claude = topology.services.endpoints.claude // {bind = "lan";};};
      hostName = "example";
      meridianEndpoint = "claude";
      jevEndpoint = "jev";
    };
  };
  missing = evaluate {
    canix-toolbelt.opencodeClaude = {
      enable = true;
      package = pkgs.emptyDirectory;
      inherit topology;
      hostName = "other";
      meridianEndpoint = "claude";
      jevEndpoint = "jev";
    };
  };
in
  assert (evaluate {}).programs.opencode.settings == {};
  assert lib.all (a: a.assertion) enabled.assertions;
  assert !(lib.all (a: a.assertion) invalid.assertions);
  assert !(builtins.tryEval (builtins.deepSeq missing.assertions true)).success;
  assert enabled.programs.opencode.settings.providers.anthropic.settings.baseURL == "http://127.0.0.1:3460/v1";
  assert (builtins.head enabled.programs.opencode.settings.plugins).options.externalBaseURL == "http://127.0.0.1:3460";
  assert (lib.last enabled.programs.opencode.settings.plugins).options.routes."http://127.0.0.1:3460/v1/messages" == "http://127.0.0.1:8792/v1/messages";
  assert enabled.systemd.user.services.jev-gateway-anthropic.Unit.Requires == ["meridian-opencode.service"];
  assert builtins.elem "HOME=/home/example" enabled.systemd.user.services.meridian-opencode.Service.Environment;
  assert builtins.elem "CLAUDE_CONFIG_DIR" enabled.systemd.user.services.meridian-opencode.Service.UnsetEnvironment;
  assert builtins.elem "ANTHROPIC_BASE_URL" enabled.systemd.user.services.meridian-opencode.Service.UnsetEnvironment;
  assert enabled.systemd.user.services.jev-gateway-anthropic.Service.LoadCredential == "typesafe:%t/agenix/typesafe";
    pkgs.writeText "opencode-claude-eval" "ok"
