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
    mkRule = r: let
      globArgs = lib.concatMapStringsSep " " (g: "-g ${lib.escapeShellArg g}") r.globs;
      pathArgs = lib.concatMapStringsSep " " lib.escapeShellArg r.paths;
    in ''
      echo ">> ${r.name}"
      if rg -n ${lib.escapeShellArg r.pattern} ${pathArgs} ${globArgs}; then
        echo ${lib.escapeShellArg r.message} >&2
        exit 1
      fi
    '';
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
            };
            globs = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [];
            };
            message = lib.mkOption {type = lib.types.str;};
          };
        });
      };
    };

    config = lib.mkIf cfg.enable {
      checks.${cfg.name} =
        pkgs.runCommand cfg.name {
          nativeBuildInputs = [pkgs.ripgrep];
        } ''
          cd ${cfg.src}
          ${lib.concatMapStringsSep "\n" mkRule cfg.rules}
          touch "$out"
        '';
    };
  });
}
