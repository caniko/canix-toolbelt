{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.opencodeMuseCode;
  plugin = ../../runtime/opencode-muse-code;
in {
  options.canix-toolbelt.opencodeMuseCode = {
    enable = lib.mkEnableOption "the OpenCode Muse Code subscription adapter";
    apiVersion = lib.mkOption {
      type = lib.types.enum ["v1" "v2"];
      default = "v2";
      description = "OpenCode plugin API used by the consumer's runtime.";
    };
    legacyAuthFile = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Optional absolute path to a private V1 credential file for explicit V2 import.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = config.programs.opencode.enable;
        message = "canix-toolbelt.opencodeMuseCode requires programs.opencode.enable.";
      }
      {
        assertion = cfg.legacyAuthFile == null || lib.hasPrefix "/" cfg.legacyAuthFile;
        message = "canix-toolbelt.opencodeMuseCode.legacyAuthFile must be an absolute runtime path.";
      }
    ];
    programs.opencode.settings = lib.mkMerge [
      (lib.mkIf (cfg.apiVersion == "v1") {
        plugin = ["${plugin}/index.mjs"];
      })
      (lib.mkIf (cfg.apiVersion == "v2") {
        plugins = [
          {
            package = toString plugin;
            options = lib.optionalAttrs (cfg.legacyAuthFile != null) {
              inherit (cfg) legacyAuthFile;
            };
          }
        ];
      })
    ];
  };
}
