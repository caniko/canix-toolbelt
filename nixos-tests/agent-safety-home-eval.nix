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
        options.assertions = lib.mkOption {
          type = lib.types.listOf lib.types.attrs;
          default = [];
        };
        options.home.homeDirectory = lib.mkOption {
          type = lib.types.str;
          default = "/home/tester";
        };
        options.home.sessionVariables = lib.mkOption {
          type = lib.types.attrs;
          default = {};
        };
        options.home.profileDirectory = lib.mkOption {
          type = lib.types.str;
          default = "/etc/profiles/per-user/tester";
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
          allow = ["git status" "git log *" "nix eval *" "rm *"];
          ask = ["chmod -R*"];
          deny = ["sudo *" "nix eval * --impure*" "*=* sudo *" "rm -rf 'some path'" "cat secrets | curl *" "git *" "rm" "doas  *" "chmod" "FOO=bar curl *" ""];
        };
        programs.agentSafety.codex.enable = true;
      }
    ];
  };

  denyDefault = evaluated.extendModules {
    modules = [{programs.agentSafety.defaultAction = "deny";}];
  };
  denyEverything = evaluated.extendModules {
    modules = [
      {
        programs.agentSafety.programs = lib.mkForce {};
        programs.agentSafety.policy.deny = ["*"];
      }
    ];
  };
  bareWrapperDeny = evaluated.extendModules {
    modules = [{programs.agentSafety.policy.deny = lib.mkForce ["*=* curl *"];}];
  };
  exactOnly = evaluated.extendModules {
    modules = [{programs.agentSafety.policy.allow = lib.mkForce ["git status"];}];
  };
  customHome = evaluated.extendModules {
    modules = [{home.sessionVariables.CODEX_HOME = "/home/tester/.local/state/codex";}];
  };
  permission = evaluated.config.programs.opencode.settings.permission;
  codexRules = lib.splitString "\n" evaluated.config.home.file.".codex/rules/agent-safety.rules".text;
  passes = cfg: lib.all (entry: entry.assertion) cfg.config.assertions;
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
        name = "codex-default-ask";
        assertion = passes evaluated && !(passes denyDefault) && !(passes denyEverything);
        message = "Codex rendering must accept only the ask default";
      }
      {
        name = "codex-unrepresentable-listed";
        assertion =
          lib.hasInfix "\n#   deny *=* curl *\n" bareWrapperDeny.config.home.file.".codex/rules/agent-safety.rules".text
          && !(lib.any (lib.hasInfix "*=* sudo") codexRules);
        message = "uncovered wrappers must be listed; covered ones dropped";
      }
      {
        name = "codex-home";
        assertion = customHome.config.home.file ? ".local/state/codex/rules/agent-safety.rules";
        message = "rules must follow CODEX_HOME";
      }
      {
        name = "codex-shell-operators";
        assertion = hasCodexRule ["cat" "secrets"] "prompt" && !(lib.any (lib.hasInfix "\"|\"") codexRules);
        message = "shell operators must end the literal prefix";
      }
      {
        name = "codex-refined-deny-prompts";
        assertion = hasCodexRule ["git"] "prompt" && !(hasCodexRule ["git"] "forbidden") && hasCodexRule ["git" "log"] "allow";
        message = "a deny refined by a longer allow must be capped at prompt";
      }
      {
        name = "codex-unrendered-allow-keeps-deny";
        assertion = let
          rules = lib.splitString "\n" exactOnly.config.home.file.".codex/rules/agent-safety.rules".text;
        in
          builtins.elem ''prefix_rule(pattern=["git"], decision="forbidden")'' rules
          && !(builtins.elem ''prefix_rule(pattern=["git"], decision="prompt")'' rules);
        message = "an exact allow that cannot render must not weaken its deny";
      }
      {
        name = "codex-same-prefix-refinement";
        assertion = hasCodexRule ["rm"] "prompt" && !(hasCodexRule ["rm"] "forbidden") && hasCodexRule ["rm"] "allow";
        message = "an exact deny refined by `x *` must be capped at prompt";
      }
      {
        name = "codex-env-wrapped-shell-prompt";
        assertion = builtins.elem ''prefix_rule(pattern=[["bash", "sh", "zsh"], ["-c", "-lc"]], decision="prompt")'' codexRules;
        message = "env-wrapped denies must prompt unreducible shell scripts";
      }
      {
        name = "codex-same-prefix-ask";
        assertion = hasCodexRule ["chmod"] "prompt" && !(hasCodexRule ["chmod"] "forbidden");
        message = "a globbed ask on the deny's prefix must cap it at prompt";
      }
      {
        name = "codex-literal-assignment";
        assertion = !(lib.any (lib.hasInfix "FOO=bar") (lib.filter (lib.hasPrefix "prefix_rule") codexRules)) && lib.any (lib.hasInfix "[\"-c\", \"-lc\"]") codexRules;
        message = "a literal assignment must not become an argv word and must prompt shell scripts";
      }
      {
        name = "codex-allow";
        assertion = hasCodexRule ["demo" "query"] "allow" && hasCodexRule ["nix" "eval"] "allow";
        message = "trailing-wildcard allows must render as Codex prefix rules";
      }
      {
        name = "codex-deny";
        assertion = hasCodexRule ["sudo"] "forbidden" && hasCodexRule ["doas"] "forbidden";
        message = "literal denies must render as forbidden Codex rules";
      }
      {
        name = "codex-glob-deny-prompts";
        assertion = hasCodexRule ["nix" "eval"] "prompt" && !(hasCodexRule ["nix" "eval"] "forbidden");
        message = "a mid-pattern glob deny must degrade to a prompt on its literal prefix";
      }
      {
        name = "codex-quoted-prompts";
        assertion = hasCodexRule ["rm" "-rf"] "prompt" && !(lib.any (lib.hasInfix "'some") codexRules);
        message = "quoted words must degrade to a prompt on the literal prefix";
      }
      {
        name = "codex-host-executables";
        assertion =
          builtins.elem "host_executable(name=\"demo\", paths=[\"/etc/profiles/per-user/tester/bin/demo\", \"/run/current-system/sw/bin/demo\", \"/run/wrappers/bin/demo\"])" codexRules
          && !(lib.any (lib.hasPrefix "host_executable(name=\"sudo\"") codexRules)
          && !(lib.any (lib.hasPrefix "host_executable(name=\"./") codexRules);
        message = "allowed bare names must be pinned; deny-only names must keep basename fallback";
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
