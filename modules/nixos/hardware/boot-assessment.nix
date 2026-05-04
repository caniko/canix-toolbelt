# Automatic boot rollback via systemd boot counting.
# Requires UKI (Unified Kernel Image) boot.
# See: https://systemd.io/AUTOMATIC_BOOT_ASSESSMENT/
{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.hardware.boot-assessment;
  inherit (lib) mkEnableOption mkOption types mkIf;
in {
  options.canix-toolbelt.hardware.boot-assessment = {
    enable = mkEnableOption "automatic boot rollback via UKI boot counting";

    tries = mkOption {
      type = types.ints.unsigned;
      default = 3;
      description = "Number of boot attempts before this generation is considered bad";
    };
  };

  config = mkIf cfg.enable {
    boot.uki.tries = cfg.tries;
  };
}
