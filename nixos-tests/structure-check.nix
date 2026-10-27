# The allowlisted facade's match and a commented match pass; dropping the
# allowlist makes the same rule fail.
{
  inputs,
  pkgs,
}: let
  check = exclude:
    (inputs.flake-parts.lib.mkFlake {inputs = {inherit (inputs) nixpkgs;};} {
      systems = [pkgs.stdenv.hostPlatform.system];
      imports = [../flake-modules/structure-check.nix];
      perSystem.canix-toolbelt.structure-check = {
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
      };
    })
    .checks
    .${
      pkgs.stdenv.hostPlatform.system
    }
    .boundary-ownership;
  failure = pkgs.testers.testBuildFailure (check []);
in
  pkgs.runCommand "structure-check" {passing = check ["lib/default.nix"];} ''
    grep -F 'lib/default.nix:3:' ${failure}/testBuildFailure.log
    grep -F 'use the path facade' ${failure}/testBuildFailure.log
    if grep -F root/module.nix ${failure}/testBuildFailure.log; then exit 1; fi
    touch "$out"
  ''
