{
  config,
  lib,
  osConfig ? null,
  ...
}: let
  inherit (lib) mkIf mkOption types;
  safety = import ../../lib/agent-safety.nix {inherit lib;};
  cfg = config.programs.agentSafety;

  programSubmodule = types.submodule {
    options = {
      autoSafe = mkOption {
        type = types.nullOr safety.autoSafeType;
        default = null;
        description = "Prompt-safe command declaration for this executable.";
      };
      description = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Human-readable program description for diagnostics.";
      };
    };
  };

  policySubmodule = types.submodule {
    options = {
      allow = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Explicitly prompt-safe Bash patterns.";
      };
      ask = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Bash patterns that always require confirmation.";
      };
      deny = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Bash patterns that must never run through the agent.";
      };
    };
  };

  guardSubmodule = types.submodule {
    options = {
      enable = mkOption {
        type = types.bool;
        default = false;
        description = "Enable a semantic destructive-command guard hook.";
      };
      package = mkOption {
        type = types.nullOr types.package;
        default = null;
        description = "Guard package supplied by the consumer (for example CC Safety Net or DCG).";
      };
      mode = mkOption {
        type = types.enum ["permissive" "strict" "fail-closed"];
        default = "strict";
        description = "Guard behavior when analysis is uncertain or malformed.";
      };
    };
  };

  sandboxSubmodule = types.submodule {
    options = {
      enable = mkOption {
        type = types.bool;
        default = false;
        description = "Enable an OS-level agent sandbox adapter.";
      };
      profile = mkOption {
        type = types.enum ["trusted" "review" "untrusted" "unattended"];
        default = "trusted";
        description = "Containment profile selected by the consumer.";
      };
    };
  };

  integratedPrograms =
    if osConfig == null
    then {}
    else
      lib.mapAttrs (_name: value: {
        inherit (value) autoSafe;
        executables = value.executables or [];
        description = "NixOS CLI tool metadata";
      }) (lib.attrByPath ["canix-toolbelt" "packages" "cliTools" "agentSafety"] {} osConfig);

  registeredPrograms = cfg.programs // integratedPrograms;

  permission = {
    edit = "allow";
    glob = "allow";
    grep = "allow";
    list = "allow";
    lsp = "allow";
    question = "allow";
    read = "allow";
    skill = "allow";
    task = "allow";
    todowrite = "allow";
    webfetch = "allow";
    websearch = "allow";
    external_directory = cfg.externalDirectories;
    bash = safety.mergeRules {
      inherit (cfg) defaultAction;
      programs = registeredPrograms;
      inherit (cfg.policy) allow ask deny;
    };
  };
in {
  options.programs.agentSafety = {
    enable = mkOption {
      type = types.bool;
      default = true;
      description = "Enable the shared agent-safety policy compiler.";
    };

    defaultAction = mkOption {
      type = types.enum ["allow" "ask" "deny"];
      default = "ask";
      description = "Action for Bash commands without a more specific rule.";
    };

    programs = mkOption {
      type = types.attrsOf programSubmodule;
      default = {};
      description = "Program safety declarations, including autoSafe rules.";
    };

    policy = mkOption {
      type = policySubmodule;
      default = {};
      description = "Central fleet-wide Bash policy. Deny rules dominate autoSafe.";
    };

    externalDirectories = mkOption {
      type = types.attrsOf (types.enum ["allow" "ask" "deny"]);
      default = {};
      description = "OpenCode external-directory access policy.";
    };

    guard = mkOption {
      type = guardSubmodule;
      default = {};
      description = "Optional semantic destructive-command guard configuration.";
    };

    sandbox = mkOption {
      type = sandboxSubmodule;
      default = {};
      description = "Optional OS containment profile configuration.";
    };
  };

  config = mkIf (cfg.enable && config.programs.opencode.enable) {
    programs.opencode.settings.permission = permission;
  };
}
