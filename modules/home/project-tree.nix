{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.projectTree;
  projectTree = import ../../lib/projectTree.nix {inherit lib;};
  authenticationType = lib.types.submodule {
    options = {
      required = lib.mkOption {
        type = lib.types.bool;
      };
      method = lib.mkOption {
        type = lib.types.enum ["token" "glab"];
      };
      tokenEnvironment = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
      };
    };
  };
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
      kind = lib.mkOption {
        type = lib.types.enum ["user" "organization" "group"];
      };
      authentication = lib.mkOption {
        type = authenticationType;
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
  repositoryPolicyType = lib.types.submodule {
    options = {
      repository = lib.mkOption {
        type = lib.types.str;
      };
      canonical = lib.mkOption {
        type = lib.types.str;
      };
      mirrors = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
      };
    };
  };
  sourceIsValid = source:
    builtins.elem "${source.provider}:${source.kind}" [
      "forgejo:user"
      "forgejo:organization"
      "github:user"
      "github:organization"
      "gitlab:user"
      "gitlab:group"
    ]
    && (
      if source.authentication.method == "token"
      then source.authentication.tokenEnvironment != null && source.authentication.tokenEnvironment != ""
      else source.provider == "gitlab" && source.authentication.tokenEnvironment == null
    );
  coordinateParts = coordinate: lib.splitString "/" coordinate;
  validCoordinate = coordinate: let
    parts = coordinateParts coordinate;
  in
    builtins.length parts >= 3 && builtins.all (part: part != "" && part != "." && part != "..") parts;
  coordinateRepository = coordinate: lib.last (coordinateParts coordinate);
  policyIsValid = policy:
    policy.repository
    != ""
    && !lib.hasInfix "/" policy.repository
    && validCoordinate policy.canonical
    && builtins.all validCoordinate policy.mirrors
    && coordinateRepository policy.canonical == policy.repository
    && builtins.all (coordinate: coordinateRepository coordinate == policy.repository) policy.mirrors
    && !(builtins.elem policy.canonical policy.mirrors)
    && builtins.length policy.mirrors == builtins.length (lib.unique policy.mirrors);
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
    repositoryPolicies = lib.mkOption {
      type = lib.types.listOf repositoryPolicyType;
      default = [];
      description = "Explicit canonical repositories and recognized mirror coordinates.";
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
      {
        assertion = builtins.all sourceIsValid cfg.sources;
        message = "canix-toolbelt.projectTree.sources contain an invalid provider/kind/authentication combination";
      }
      {
        assertion = builtins.all policyIsValid cfg.repositoryPolicies;
        message = "canix-toolbelt.projectTree.repositoryPolicies must use one leaf and distinct canonical/mirror coordinates";
      }
      {
        assertion =
          builtins.length cfg.repositoryPolicies
          == builtins.length (lib.unique (map (policy: policy.repository) cfg.repositoryPolicies));
        message = "canix-toolbelt.projectTree.repositoryPolicies must use unique repository leaves";
      }
    ];
    xdg.configFile."canix/project-tree.json".text = builtins.toJSON {
      inherit (projectTree) schemaVersion layout;
      inherit (cfg) root controlRemote sources exclusions repositoryPolicies;
    };
    home.packages = [pkgs.git];
  };
}
