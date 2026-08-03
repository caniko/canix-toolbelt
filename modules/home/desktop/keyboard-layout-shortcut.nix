{
  config,
  cosmicLib,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.keyboard-layout-shortcut;
  inherit (cosmicLib.cosmic) mkRON;
  layout =
    lib.attrByPath [
      "wayland"
      "desktopManager"
      "cosmic"
      "compositor"
      "xkb_config"
      "layout"
    ] ""
    config;
  layouts = lib.filter (entry: lib.trim entry != "") (lib.splitString "," layout);
in {
  options.canix-toolbelt.keyboard-layout-shortcut = {
    enable = lib.mkEnableOption "the keyboard-layout switch shortcut";

    shortcut = lib.mkOption {
      type = lib.types.str;
      default = "Super+Delete";
      description = "COSMIC key binding used to switch keyboard layouts.";
    };
  };

  config = lib.mkIf (cfg.enable && builtins.length layouts > 1) {
    wayland.desktopManager.cosmic.shortcuts = [
      {
        action = mkRON "enum" {
          value = [
            (mkRON "enum" "InputSourceSwitch")
          ];
          variant = "System";
        };
        description = mkRON "optional" "Switch keyboard layout";
        key = cfg.shortcut;
      }
    ];
  };
}
