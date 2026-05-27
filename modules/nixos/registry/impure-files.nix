{
  config,
  lib,
  ...
}: let
  inherit (lib) filterAttrs mapAttrsToList mkOption nameValuePair types;

  hostname = config.networking.hostName;
  cfg = config.canix-toolbelt.impureFiles;

  effectiveBaseDir = let
    hostOverride = cfg.impureBaseDir.perHost.${hostname} or null;
  in
    if hostOverride != null
    then hostOverride
    else cfg.impureBaseDir.default;

  impureFileSubmodule = types.submodule ({config, ...}: {
    options = {
      name = mkOption {
        type = types.str;
        description = "Unique identifier for this impure file";
      };

      path = mkOption {
        type = types.str;
        example = "secrets/api-key";
        description = "Path relative to the impure base directory";
      };

      envVar = mkOption {
        type = types.str;
        example = "API_KEY_FILE";
        description = "Environment variable name pointing to this file's absolute path";
      };

      description = mkOption {
        type = types.str;
        description = "Human-readable description of this file's purpose";
      };

      absolutePath = mkOption {
        type = types.str;
        readOnly = true;
        default = "${effectiveBaseDir}/${config.path}";
        description = "The absolute path to this file (baseDir + relativePath)";
      };
    };
  });

  envVars =
    filterAttrs (_: fileData: fileData.envVar != null) cfg.files;
in {
  options.canix-toolbelt.impureFiles = {
    impureBaseDir = {
      default = mkOption {
        type = types.str;
        default = "/etc/canix/impure";
        description = "Default base directory for impure files.";
      };

      perHost = mkOption {
        type = types.attrsOf types.str;
        default = {};
        description = "Per-host base directory overrides for impure files.";
      };
    };

    baseDir = mkOption {
      type = types.str;
      readOnly = true;
      default = effectiveBaseDir;
      description = "Base directory for impure files (computed per-host)";
    };

    files = mkOption {
      type = types.attrsOf impureFileSubmodule;
      default = {};
      description = "Registry of impure files with their paths and environment variables";
    };
  };

  config.environment.variables =
    {
      CANIX_IMPURE_DIR = effectiveBaseDir;
    }
    // (lib.listToAttrs (mapAttrsToList (
        _: fileData:
          nameValuePair fileData.envVar fileData.absolutePath
      )
      envVars));
}
