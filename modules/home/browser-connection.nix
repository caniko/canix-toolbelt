{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.browserConnection;
  adapter = (import ../../lib/browserConnection.nix {inherit lib;}).mkAdapter {
    inherit pkgs;
    inherit (cfg) browser headless;
  };
in {
  options.canix-toolbelt.browserConnection = {
    enable = lib.mkEnableOption "default-browser automation connections";
    browser = lib.mkOption {
      type = lib.types.nullOr (lib.types.submodule {
        options = {
          family = lib.mkOption {type = lib.types.enum ["firefox" "chromium"];};
          executable = lib.mkOption {type = lib.types.str;};
          arguments = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [];
          };
        };
      });
      default = null;
      description = "Explicit browser override. Null discovers the user's XDG default at connection time; unsupported defaults fail without substituting another browser.";
    };
    headless = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Run the selected browser without a window, using a separate automation profile.";
    };
    adapters.opencode = lib.mkOption {
      type = lib.types.attrs;
      readOnly = true;
      description = "OpenCode native-browser host command and implemented operation inventory.";
    };
  };
  config = {
    home.packages = lib.mkIf cfg.enable [adapter.package];
    canix-toolbelt.browserConnection.adapters.opencode =
      if cfg.enable
      then {inherit (adapter) command operations;}
      else {};
  };
}
