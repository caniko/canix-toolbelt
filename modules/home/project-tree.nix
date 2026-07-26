{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.projectTree;
  projectTree = import ../../lib/projectTree.nix {inherit lib;};
  sourceType = lib.types.submodule {
    options = {
      provider = lib.mkOption {
        type = lib.types.enum ["forgejo" "github" "gitlab"];
      };
      host = lib.mkOption {
        type = lib.types.str;
      };
      namespace = lib.mkOption {
        type = lib.types.str;
      };
      includeSubgroups = lib.mkOption {
        type = lib.types.bool;
        default = false;
      };
    };
  };
  exclusionType = lib.types.submodule {
    options = {
      coordinate = lib.mkOption {
        type = lib.types.str;
      };
      reason = lib.mkOption {
        type = lib.types.str;
      };
    };
  };
in {
  options.canix-toolbelt.projectTree = {
    enable = lib.mkEnableOption "the standardized project checkout tree";
    root = lib.mkOption {
      type = lib.types.str;
      description = "Absolute project workspace root.";
    };
    controlRemote = lib.mkOption {
      type = lib.types.str;
      description = "Git remote of the projects control superproject.";
    };
    sources = lib.mkOption {
      type = lib.types.listOf sourceType;
      default = [];
      description = "Forge namespaces whose non-archived repositories are desired checkouts.";
    };
    exclusions = lib.mkOption {
      type = lib.types.listOf exclusionType;
      default = [];
      description = "Reasoned repository exclusions keyed by canonical forge coordinate.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.hasPrefix "/" cfg.root;
        message = "canix-toolbelt.projectTree.root must be absolute";
      }
      {
        assertion = cfg.sources != [];
        message = "canix-toolbelt.projectTree.sources must declare at least one managed namespace";
      }
    ];
    xdg.configFile."canix/project-tree.json".text = builtins.toJSON {
      inherit (projectTree) schemaVersion layout;
      inherit (cfg) root controlRemote sources exclusions;
    };
    home.packages = [pkgs.git];
  };
}
