{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.opencodeEnvironment;
in {
  options.canix-toolbelt.opencodeEnvironment = {
    enable = lib.mkEnableOption "the Toolbelt OpenCode project environment integration";
    package = lib.mkOption {
      type = lib.types.package;
      description = "The generic harbor-llm payload supplied by the consumer.";
    };
    options = lib.mkOption {
      type = lib.types.attrsOf lib.types.anything;
      default = {};
      description = "Consumer project roots, backend and policy passed to Harbor.";
    };
  };
  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = config.programs.opencode.enable;
        message = "canix-toolbelt.opencodeEnvironment requires programs.opencode.enable.";
      }
    ];
    programs.opencode.settings.plugins = [
      {
        package = toString ../../runtime/opencode-environment;
        options =
          {
            direnv = lib.getExe pkgs.direnv;
            nix = lib.getExe pkgs.nix;
            setsid = "${pkgs.util-linux}/bin/setsid";
            flock = "${pkgs.util-linux}/bin/flock";
            system = pkgs.stdenv.hostPlatform.system;
          }
          // cfg.options
          // {
            plugin = "${cfg.package}/lib/harbor-llm/src/project-environment-v2.mjs";
          };
      }
    ];
  };
}
