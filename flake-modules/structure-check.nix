# Generic boundary-ownership check using ripgrep. Consumers declare a list of
# forbidden patterns to grep for under their flake source; if any match, the
# check fails.
#
# Usage in a consuming flake:
#
#   imports = [inputs.canix-toolbelt.flakeModules.structure-check];
#   perSystem = {pkgs, ...}: {
#     canix-toolbelt.structure-check = {
#       enable = true;
#       src = ./.;
#       rules = [
#         {
#           name = "no-home-imports-in-root";
#           pattern = "\\.\\./.*home/";
#           paths = ["root"];
#           globs = ["*.nix"];
#           message = "root/ must not import home/ paths";
#         }
#         {
#           # Allowlist files with `exclude`; `pcre2` enables lookarounds.
#           name = "no-deep-relative-paths";
#           pattern = "^(?!\\s*#).*\\.\\./\\.\\./";
#           pcre2 = true;
#           paths = ["root" "lib"];
#           globs = ["*.nix"];
#           exclude = ["lib/default.nix"];
#           message = "use the path facade instead of ../../";
#         }
#       ];
#     };
#   };
{
  lib,
  flake-parts-lib,
  ...
}: {
  options.perSystem = flake-parts-lib.mkPerSystemOption ({
    config,
    pkgs,
    ...
  }: let
    cfg = config.canix-toolbelt.structure-check;
    mkStructureCheck = import ../lib/mkStructureCheck.nix {inherit lib;};
  in {
    options.canix-toolbelt.structure-check = {
      enable = lib.mkEnableOption "boundary-ownership check";

      src = lib.mkOption {
        type = lib.types.path;
        description = "Source tree to scan.";
      };

      name = lib.mkOption {
        type = lib.types.str;
        default = "boundary-ownership";
        description = "Name of the resulting check derivation.";
      };

      rules = lib.mkOption {
        description = "Forbidden patterns. Any match fails the check.";
        default = [];
        type = lib.types.listOf (lib.types.submodule {
          options = {
            name = lib.mkOption {type = lib.types.str;};
            pattern = lib.mkOption {type = lib.types.str;};
            paths = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = ["."];
              description = "Directories within src to scan. File paths are rejected; select files with globs under their parent directory.";
            };
            globs = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [];
            };
            exclude = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [];
              example = ["lib/default.nix"];
              description = "Source-root-relative files or directories allowed to match (an allowlist).";
            };
            pcre2 = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Use PCRE2 syntax, e.g. lookarounds to skip comment lines.";
            };
            message = lib.mkOption {type = lib.types.str;};
          };
        });
      };
    };

    config = lib.mkIf cfg.enable {
      checks.${cfg.name} = mkStructureCheck {inherit pkgs;} {
        inherit (cfg) name src rules;
      };
    };
  });
}
