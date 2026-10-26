{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  evaluated = lib.evalModules {
    specialArgs = {osConfig = null;};
    modules = [
      {
        options.programs.opencode = {
          enable = lib.mkOption {
            type = lib.types.bool;
            default = false;
          };
          settings = lib.mkOption {
            type = lib.types.submodule {freeformType = lib.types.attrs;};
            default = {};
          };
        };
        options.home.file = lib.mkOption {
          type = lib.types.attrsOf lib.types.attrs;
          default = {};
        };
      }
      ../modules/home/agent-safety.nix
      {
        programs.opencode.enable = true;
        programs.agentSafety.programs.demo.autoSafe = {
          commands = ["query *"];
          executables = ["demo"];
        };
        programs.agentSafety.policy = {
          allow = ["git status" "nix eval *"];
          deny = ["sudo *" "nix eval * --impure*" "*=* sudo *"];
        };
        programs.agentSafety.codex.enable = true;
      }
    ];
  };

  permission = evaluated.config.programs.opencode.settings.permission;
  codexRules = lib.splitString "\n" evaluated.config.home.file.".codex/rules/agent-safety.rules".text;
  hasCodexRule = tokens: decision:
    builtins.elem "prefix_rule(pattern=[${lib.concatMapStringsSep ", " builtins.toJSON tokens}], decision=\"${decision}\")" codexRules;
in
  mkEvalCheck {
    name = "agent-safety-home-eval";
    resultMessage = "Home Manager agent-safety rendering passed";
    assertions = [
      {
        name = "open-code-enabled";
        assertion = permission.bash."*" == "ask";
        message = "unknown Bash commands must remain ask";
      }
      {
        name = "program-rule";
        assertion = permission.bash."demo query *" == "allow";
        message = "registered autoSafe commands must render into OpenCode";
      }
      {
        name = "deny-rule";
        assertion = permission.bash."sudo *" == "deny";
        message = "central deny rules must render into OpenCode";
      }
      {
        name = "codex-allow";
        assertion = hasCodexRule ["demo" "query"] "allow" && hasCodexRule ["nix" "eval"] "allow";
        message = "trailing-wildcard allows must render as Codex prefix rules";
      }
      {
        name = "codex-deny";
        assertion = hasCodexRule ["sudo"] "forbidden";
        message = "literal denies must render as forbidden Codex rules";
      }
      {
        name = "codex-glob-deny-prompts";
        assertion = hasCodexRule ["nix" "eval"] "prompt" && !(hasCodexRule ["nix" "eval"] "forbidden");
        message = "a mid-pattern glob deny must degrade to a prompt on its literal prefix";
      }
      {
        name = "codex-skips-widening";
        assertion =
          !(lib.any (lib.hasInfix "\"git\", \"status\"") codexRules)
          && !(lib.any (lib.hasInfix "\"*") codexRules)
          && !(lib.any (lib.hasInfix "\"/nix/store/") codexRules);
        message = "exact allows and leading-glob patterns must not render";
      }
    ];
  }
