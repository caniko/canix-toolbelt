# The allowlisted facade's match and a commented match pass; dropping the
# allowlist makes the same rule fail.
{
  inputs,
  pkgs,
}: let
  check = exclude: extra:
    (inputs.flake-parts.lib.mkFlake {
        # A minimal stand-in; the fixture flake has no inputs or outputs.
        inputs.self.inputs = {};
      } {
        systems = [pkgs.stdenv.hostPlatform.system];
        imports = [../flake-modules/structure-check.nix];
        perSystem._module.args.pkgs = pkgs;
        perSystem.canix-toolbelt.structure-check =
          {
            enable = true;
            src = ./fixtures/structure-check;
            rules = [
              {
                name = "no-deep-relative-paths";
                pattern = "^(?!\\s*#).*\\.\\./\\.\\./";
                pcre2 = true;
                paths = ["root" "lib"];
                globs = ["*.nix"];
                inherit exclude;
                message = "use the path facade instead of ../../";
              }
            ];
          }
          // extra;
      })
    .checks
    .${
      pkgs.stdenv.hostPlatform.system
    }
    .boundary-ownership;
  failure = pkgs.testers.testBuildFailure (check [] {});
  rule = name: pattern: {
    inherit name pattern;
    message = "${name} reported";
  };
  scan = options:
    check [] ({
        rules = [
          (rule "hidden-rule" "hidden-violation")
          (rule "ignored-rule" "gitignored-violation")
        ];
      }
      // options);
  first = pkgs.testers.testBuildFailure (scan {
    includeHidden = true;
    noIgnore = true;
  });
  all = pkgs.testers.testBuildFailure (scan {
    includeHidden = true;
    noIgnore = true;
    reportAll = true;
  });
in
  pkgs.runCommand "structure-check" {
    passing = check ["lib/default.nix"] {};
    defaultScan = scan {};
  } ''
    grep -F 'lib/default.nix:3:' ${failure}/testBuildFailure.log
    grep -F 'use the path facade' ${failure}/testBuildFailure.log
    if grep -F root/module.nix ${failure}/testBuildFailure.log; then exit 1; fi
    grep -F 'hidden-rule reported' ${first}/testBuildFailure.log
    if grep -F 'ignored-rule reported' ${first}/testBuildFailure.log; then exit 1; fi
    grep -F 'hidden-rule reported' ${all}/testBuildFailure.log
    grep -F 'ignored-rule reported' ${all}/testBuildFailure.log
    touch "$out"
  ''
