{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  mkEvalCheck = inputs.self.lib.mkEvalCheck {inherit pkgs;};
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
  rejected = builtins.tryEval ((mkEvalCheck {
    name = "eval-check-helper-rejection";
    assertions = [
      {
        name = "broken-contract";
        assertion = false;
        message = "must reject";
      }
    ];
  }).drvPath);
  scriptFor = rule: let
    fixture = inputs.flake-parts.lib.mkFlake {inherit inputs;} {
      systems = [pkgs.stdenv.hostPlatform.system];
      imports = [../flake-modules/structure-check.nix];
      perSystem.canix-toolbelt.structure-check = {
        enable = true;
        src = ./fixtures/structure-check;
        rules = [
          ({
              name = "fixture-rule";
              message = "fixture source violation";
              globs = ["*.nix"];
            }
            // rule)
        ];
      };
    };
  in
    fixture.checks.${pkgs.stdenv.hostPlatform.system}.boundary-ownership.buildCommand;
  directScript =
    (inputs.self.lib.mkStructureCheck {inherit pkgs;} {
      name = "boundary-ownership";
      src = ./fixtures/structure-check;
      rules = [
        {
          name = "fixture-rule";
          message = "fixture source violation";
          globs = ["*.nix"];
          pattern = "absent";
        }
      ];
    }).buildCommand;
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
  ];
in
  assert !rejected.success;
  assert directScript == scriptFor {pattern = "absent";};
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
