{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.homeManagerBackupRepair;
  repair = import ../../nix/home-manager-backup-repair.nix {
    inherit pkgs;
    inherit (cfg) tag;
  };
in {
  options.canix-toolbelt.homeManagerBackupRepair = {
    enable = lib.mkEnableOption ''
      moving stale Home Manager `*.<backupFileExtension>` backups aside during
      NixOS activation, so Home Manager does not refuse to clobber them'';

    tag = lib.mkOption {
      type = lib.types.strMatching "[A-Za-z0-9_.-]+";
      default = "toolbelt";
      example = "canix";
      description = "Log prefix and infix of the moved name: `<backup>.<tag>-<UTC stamp>`.";
    };
  };

  config = lib.mkIf cfg.enable {
    # Home Manager activation runs after NixOS activation. Repairing stale
    # backups here keeps the root-only filesystem mutation in activation.
    system.activationScripts.homeManagerBackupRepair = {
      deps = ["users"];
      supportsDryActivation = false;
      text = ''
        ${lib.getExe repair} "$systemConfig"
      '';
    };
  };
}
