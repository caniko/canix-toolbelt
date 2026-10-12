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

  disjointStoreCommands = builtins.tryEval (builtins.deepSeq (safety.mergeRules {
      programs.demo.autoSafe = {
        commands = ["query *"];
        wrappers = {
          environmentAssignments = false;
          nixStorePaths = false;
          resultBinPaths = false;
        };
      };
      deny = ["/nix/store/*/bin/other delete *"];
    })
    true);

  # Both store globs match this whole command under OpenCode's native matcher:
  # /nix/store/hash-zed/bin/zed query /bin/other delete x
  # The path wildcard is not limited to one store directory or the executable.
  storeArgumentConflict = builtins.tryEval (builtins.deepSeq (safety.mergeRules {
      programs.zed.autoSafe = {commands = ["query *"];};
      deny = ["/nix/store/*/bin/other delete *"];
    })
    true);

  envStoreArgumentConflict = builtins.tryEval (builtins.deepSeq (safety.mergeRules {
      programs.zed.autoSafe = {commands = ["query *"];};
      deny = ["*=* /nix/store/*/bin/other delete *"];
    })
    true);

  storeConflict = builtins.tryEval (builtins.deepSeq (safety.mergeRules {
      programs.demo.autoSafe = {commands = ["query *"];};
      deny = ["/nix/store/*/bin/demo query --delete *"];
    })
    true);
in
  mkEvalCheck {
    name = "agent-safety-eval";
    resultMessage = "autoSafe normalization, wrapper expansion, and deny dominance passed";
    assertions = [
      {
        name = "disjoint-store-commands";
        assertion = disjointStoreCommands.success;
        message = "an unwrapped command remains disjoint from a store-prefixed deny";
      }
      {
        name = "store-argument-overlap";
        assertion = !storeArgumentConflict.success;
        message = "a store wildcard can consume command arguments before a competing executable suffix";
      }
      {
        name = "env-store-argument-overlap";
        assertion = !envStoreArgumentConflict.success;
        message = "environment-prefixed store wildcards retain whole-command overlap";
      }
      {
        name = "overlapping-store-commands";
        assertion = !storeConflict.success;
        message = "a store-prefixed autoSafe command must still conflict with its destructive form";
      }
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
        name = "combined-env-path-wrappers";
        assertion =
          wildcard."*=* /nix/store/*/bin/demo *"
          == "allow"
          && wildcard."*=* ./result/bin/demo *" == "allow"
          && !(builtins.hasAttr "*=* ./result/bin/demo query *" narrow);
        message = "environment wrappers must compose with enabled executable paths";
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
