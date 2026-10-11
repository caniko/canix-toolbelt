{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkEnableOption mkOption types;
  cfg = config.canix-toolbelt.headroom;
  endpointType = types.submodule {
    options = {
      url = mkOption {
        type = types.str;
        description = "Local Headroom proxy base URL.";
      };
      unit = mkOption {
        type = types.str;
        description = "Infernix-managed proxy service unit.";
      };
    };
  };
  endpoints = builtins.attrValues cfg.opencodeGateways ++ lib.optional cfg.claude.enable cfg.claude.endpoint;
  codexWrapper = pkgs.writeTextFile {
    name = "headroom-codex";
    destination = "/bin/codex";
    executable = true;
    text = "#!${cfg.package.pythonInterpreter}\n" + lib.replaceStrings ["@codex@" "@port@"] [(lib.getExe cfg.codex.package) (toString cfg.codex.port)] (builtins.readFile ../../runtime/headroom/codex.py);
    meta.priority = -10;
  };
in {
  imports = [./opencode-jev.nix];
  # Extend each declared gateway in place. Defining another gateways attrset
  # from opencodeGateways would recurse when consumers derive endpoints from
  # the existing gateway inventory.
  options.canix-toolbelt.opencodeJev.gateways = mkOption {
    type = types.attrsOf (types.submodule ({name, ...}: {
      config = lib.mkIf (cfg.enable && builtins.hasAttr name cfg.opencodeGateways) {
        forwardUrl = "${cfg.opencodeGateways.${name}.url}/v1";
        requires = lib.mkAfter [cfg.opencodeGateways.${name}.unit];
      };
    }));
  };
  options.canix-toolbelt.headroom = {
    enable = mkEnableOption "Headroom adapters for coding agents";
    package = mkOption {
      type = types.package;
      description = "Infernix Headroom package with its immutable pythonInterpreter passthrough.";
    };
    opencodeGateways = mkOption {
      type = types.attrsOf endpointType;
      default = {};
      description = "Proxy endpoints keyed by existing opencodeJev gateway identity. Jev runs before compression.";
    };
    codex = {
      enable = mkEnableOption "session-scoped Codex compression without configuration writes";
      package = mkOption {
        type = types.package;
        default = pkgs.codex;
      };
      port = mkOption {
        type = types.port;
        default = 8787;
      };
    };
    claude = {
      enable = mkEnableOption "Claude Code compression before its existing router";
      endpoint = mkOption {type = endpointType;};
    };
  };
  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.opencodeGateways == {} || config.canix-toolbelt.opencodeJev.enable;
        message = "Headroom OpenCode adapters require opencodeJev.";
      }
      {
        assertion = lib.all (name: builtins.hasAttr name config.canix-toolbelt.opencodeJev.gateways) (builtins.attrNames cfg.opencodeGateways);
        message = "Headroom may redirect only declared Jev gateways.";
      }
      {
        assertion = lib.all (endpoint: builtins.match "http://127[.]0[.]0[.]1:[0-9]+" endpoint.url != null) endpoints;
        message = "Headroom client endpoints must be HTTP loopback origins.";
      }
      {
        assertion = !cfg.codex.enable || cfg.package ? pythonInterpreter;
        message = "Headroom Codex requires the pinned package's pythonInterpreter.";
      }
    ];
    home.packages = lib.optional cfg.codex.enable codexWrapper;
    home.sessionVariables = lib.optionalAttrs cfg.claude.enable {ANTHROPIC_BASE_URL = cfg.claude.endpoint.url;};
  };
}
