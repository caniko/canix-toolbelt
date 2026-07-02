{
  inputs,
  lib,
  flake-parts-lib,
  ...
}: {
  imports = [inputs.git-hooks.flakeModule];

  options.perSystem = flake-parts-lib.mkPerSystemOption ({config, ...}: let
    cfg = config.canix-toolbelt.pre-commit;
  in {
    options.canix-toolbelt.pre-commit.mypy = {
      enable = lib.mkEnableOption "mypy pre-commit hook";

      files = lib.mkOption {
        type = lib.types.str;
        default = "\\.py$";
        description = "Regex selecting Python files for the mypy pre-commit hook.";
      };

      excludes = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "Regexes excluded from the mypy pre-commit hook.";
      };

      args = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = "Additional arguments passed to mypy.";
      };

      passFilenames = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether pre-commit passes matched filenames to mypy.";
      };

      binPath = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Optional mypy executable path, for projects that need custom stubs or plugins.";
      };
    };

    config.pre-commit = {
      check.enable = true;
      settings = {
        install.enable = true;
        hooks = {
          treefmt.enable = true;
          mypy = lib.mkIf cfg.mypy.enable {
            enable = true;
            inherit (cfg.mypy) args excludes files;
            pass_filenames = cfg.mypy.passFilenames;
            settings.binPath = lib.mkIf (cfg.mypy.binPath != null) cfg.mypy.binPath;
          };
        };
      };
    };
  });
}
