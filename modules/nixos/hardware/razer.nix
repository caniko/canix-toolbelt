{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.hardware.razer;
in {
  options.canix-toolbelt.hardware.razer = {
    enable = lib.mkEnableOption "Razer hardware support";

    user = lib.mkOption {
      type = lib.types.str;
      description = "The user to add to the plugdev group and openrazer users";
    };
  };

  config = lib.mkIf cfg.enable {
    users.users.${cfg.user}.extraGroups = ["plugdev"];

    environment.systemPackages = with pkgs; [
      (polychromatic.overrideAttrs (oldAttrs: {
        # 'od' is in coreutils. We need to add it to the program's PATH.
        postFixup =
          (oldAttrs.postFixup or "")
          + ''
            for program in $out/bin/*; do
              wrapProgram $program --prefix PATH : ${lib.makeBinPath [coreutils]}
            done
          '';
      }))
    ];

    hardware.openrazer = {
      enable = true;
      users = [cfg.user];
    };
  };
}
