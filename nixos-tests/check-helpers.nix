{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  mkEvalCheck = inputs.self.lib.mkEvalCheck {inherit pkgs;};
  mkStructureCheck = inputs.self.lib.mkStructureCheck {inherit pkgs;};
  fixtureRoot = ./fixtures/structure-check;
  fixtureRule = {
    name = "fixture-rule";
    message = "fixture source violation";
    globs = ["*.nix"];
  };
  checkFor = src: rule:
    mkStructureCheck {
      name = "boundary-ownership";
      inherit src;
      rules = [(fixtureRule // rule)];
    };
  receipt = mkEvalCheck {
    name = "eval-check-helper-receipt";
    assertions = [
      {
        name = "named-contract";
        assertion = true;
        message = "preserved diagnostic";
      }
    ];
    runtimeScript = ''touch "$out/runtime-ok"'';
  };
  rejected =
    builtins.tryEval
    (mkEvalCheck {
      name = "eval-check-helper-rejection";
      assertions = [
        {
          name = "broken-contract";
          assertion = false;
          message = "must reject";
        }
      ];
    }).drvPath;
  scriptFor = rule: let
    fixture = inputs.flake-parts.lib.mkFlake {inherit inputs;} {
      systems = [pkgs.stdenv.hostPlatform.system];
      imports = [../flake-modules/structure-check.nix];
      perSystem.canix-toolbelt.structure-check = {
        enable = true;
        src = fixtureRoot;
        rules = [(fixtureRule // rule)];
      };
    };
  in
    fixture.checks.${pkgs.stdenv.hostPlatform.system}.boundary-ownership.buildCommand;
  directScript = (checkFor fixtureRoot {pattern = "absent";}).buildCommand;
  fileRule = {
    pattern = "forbidden";
    paths = ["fixture.nix"];
    globs = ["*.yaml"];
  };
  fileRejected = builtins.tryEval (checkFor fixtureRoot fileRule).drvPath;
  moduleFileRejected = builtins.tryEval (scriptFor fileRule);
  scopedCheck = src:
    checkFor src {
      pattern = "absent";
      paths = ["scoped/nested"];
    };
  withoutIgnore = path:
    lib.fileset.toSource {
      root = fixtureRoot;
      fileset = lib.fileset.difference fixtureRoot (fixtureRoot + "/${path}");
    };
  cases = [
    {
      name = "clean";
      rule.pattern = "absent";
      status = 0;
    }
    {
      name = "violation";
      rule.pattern = "forbidden";
      status = 1;
    }
    {
      name = "scanner-error";
      rule.pattern = "[";
      status = 2;
    }
    {
      name = "leading-dash";
      rule.pattern = "-forbidden";
      status = 1;
    }
    {
      name = "empty-paths";
      rule = {
        pattern = "forbidden";
        paths = [];
      };
      status = 1;
    }
    {
      name = "excluded";
      rule = {
        pattern = "forbidden";
        globs = ["*.nix" "!fixture.nix"];
      };
      status = 0;
    }
    {
      name = "gitignore";
      rule = {
        pattern = "gitignored-violation";
        paths = ["scoped"];
        globs = [];
      };
      status = 0;
    }
    {
      name = "ancestor-ignore";
      rule = {
        pattern = "ancestor-violation";
        paths = ["scoped/nested"];
        globs = [];
      };
      status = 0;
    }
    {
      name = "glob-reinclude";
      rule = {
        pattern = "gitignored-violation";
        paths = ["scoped"];
        globs = ["gitignored.nix"];
      };
      status = 1;
    }
    {
      name = "scoped-violation";
      rule = {
        pattern = "allowed";
        paths = ["scoped/nested"];
        globs = [];
      };
      status = 1;
    }
  ];
in
  assert !rejected.success;
  assert directScript == scriptFor {pattern = "absent";};
  assert lib.assertMsg (!fileRejected.success && !moduleFileRejected.success)
  "check-helpers: file-valued scopes must reject instead of bypassing globs";
  assert lib.assertMsg ((scopedCheck fixtureRoot).drvPath == (scopedCheck (withoutIgnore "docs/.ignore")).drvPath)
  "check-helpers: unrelated ignore files must not invalidate scoped checks";
  assert lib.assertMsg ((scopedCheck fixtureRoot).drvPath != (scopedCheck (withoutIgnore "scoped/.ignore")).drvPath)
  "check-helpers: ancestor ignore files must remain source dependencies";
  assert lib.assertMsg ((scopedCheck fixtureRoot).drvPath != (scopedCheck (withoutIgnore ".gitignore")).drvPath)
  "check-helpers: root ignore files must remain source dependencies";
    pkgs.runCommand "check-helpers" {nativeBuildInputs = [pkgs.bash pkgs.ripgrep pkgs.jq];} ''
      set -euo pipefail
      jq -e '.[0] == {name: "named-contract", message: "preserved diagnostic"}' ${receipt}/assertions.json
      test -f ${receipt}/runtime-ok
      ${lib.concatMapStringsSep "\n" (case: ''
          status=0
          out="$TMPDIR/${case.name}.out" bash -e ${pkgs.writeText "structure-${case.name}.sh" (scriptFor case.rule)} > "$TMPDIR/${case.name}.log" 2>&1 || status=$?
          if [ "$status" -ne ${toString case.status} ]; then
            cat "$TMPDIR/${case.name}.log" >&2
            echo "${case.name}: expected ${toString case.status}, got $status" >&2
            exit 1
          fi
        '')
        cases}
      grep -F 'fixture source violation' "$TMPDIR/violation.log"
      grep -F 'scanner failed' "$TMPDIR/scanner-error.log"
      touch "$out"
    ''
