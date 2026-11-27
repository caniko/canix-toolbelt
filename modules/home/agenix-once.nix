# Units that want agenix.service would rerun its oneshot on every restart and
# rewrite secrets that may have been swapped live. Decrypt once per generation
# instead: Home Manager restarts the unit when its mount script, and so its
# secrets, change.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.agenix-once;
  # A stop of the now-active unit during a switch propagates through these and
  # leaves an unchanged dependent stopped; Wants= + After= is enough.
  hard = ["Requires" "Requisite" "BindsTo" "PartOf"];
  offenders = lib.attrNames (lib.filterAttrs (_: unit:
    lib.any (key: lib.elem "agenix.service" (lib.toList (unit.Unit.${key} or []))) hard)
  config.systemd.user.services);
in {
  options.canix-toolbelt.agenix-once = {
    enable = lib.mkEnableOption "decrypting Home Manager agenix secrets once per generation (RemainAfterExit) while rejecting hard systemd dependencies on agenix.service";
  };

  config = lib.mkIf (cfg.enable && (config.age.enable or false) && pkgs.stdenv.hostPlatform.isLinux) {
    systemd.user.services.agenix.Service.RemainAfterExit = true;
    assertions = [
      {
        assertion = offenders == [];
        message = "User units ${toString offenders} hard-depend on agenix.service; use Wants= and After= instead.";
      }
    ];
  };
}
