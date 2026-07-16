{pkgs}: let
  inherit (pkgs) lib;
  safety = (import ../lib {inherit lib;}).agentSafety;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  wildcard = safety.mergeRules {
    programs.demo.autoSafe = "*";
  };

  narrow = safety.mergeRules {
    programs.demo.autoSafe = {
      commands = ["query *"];
      executables = ["demo" "demo-admin"];
      wrappers = {
        environmentAssignments = false;
        nixStorePaths = false;
        resultBinPaths = false;
      };
    };
  };

  conflict = builtins.tryEval (builtins.deepSeq (safety.mergeRules {
      programs.demo.autoSafe = {commands = ["sudo *"];};
      deny = ["demo sudo *"];
    })
    true);

  overlappingConflict = builtins.tryEval (builtins.deepSeq (safety.mergeRules {
      programs.demo.autoSafe = {commands = ["query *"];};
      deny = ["demo query --delete *"];
    })
    true);
in
  mkEvalCheck {
    name = "agent-safety-eval";
    resultMessage = "autoSafe normalization, wrapper expansion, and deny dominance passed";
    assertions = [
      {
        name = "wildcard-command";
        assertion = wildcard."demo" == "allow" && wildcard."demo *" == "allow";
        message = "the string shorthand must allow no-argument and subcommand invocations";
      }
      {
        name = "wildcard-env-wrapper";
        assertion = wildcard."*=* demo" == "allow" && wildcard."*=* demo *" == "allow";
        message = "wildcard declarations must include environment-prefix wrappers";
      }
      {
        name = "narrow-command";
        assertion = narrow."demo query *" == "allow" && narrow."demo-admin query *" == "allow";
        message = "structured declarations must render all declared executable aliases";
      }
      {
        name = "wrapper-disable";
        assertion =
          !(builtins.hasAttr "*=* demo query *" narrow)
          && !(builtins.hasAttr "/nix/store/*/bin/demo query *" narrow);
        message = "structured wrapper flags must be honored";
      }
      {
        name = "deny-dominance";
        assertion = conflict.success == false;
        message = "an autoSafe rule must never shadow an explicit deny";
      }
      {
        name = "deny-overlap-dominance";
        assertion = overlappingConflict.success == false;
        message = "a broad autoSafe pattern must not overlap a narrower deny";
      }
    ];
  }
