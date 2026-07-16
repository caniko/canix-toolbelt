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
      }
      ../modules/home/agent-safety.nix
      {
        programs.opencode.enable = true;
        programs.agentSafety.programs.demo.autoSafe = {
          commands = ["query *"];
          executables = ["demo"];
        };
        programs.agentSafety.policy.deny = ["sudo *"];
      }
    ];
  };

  permission = evaluated.config.programs.opencode.settings.permission;
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
    ];
  }
