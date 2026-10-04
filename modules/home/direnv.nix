{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.direnv;
in {
  options.canix-toolbelt.direnv = {
    enable = lib.mkEnableOption "nix-direnv with batched, deduplicated input GC roots";
    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.nix-direnv;
      defaultText = lib.literalExpression "pkgs.nix-direnv";
      description = "Base nix-direnv package; existing patches are preserved.";
    };
  };

  config = lib.mkIf cfg.enable {
    programs.direnv = {
      enable = true;
      nix-direnv = {
        enable = true;
        package = cfg.package.overrideAttrs (old: {
          patches = (old.patches or []) ++ [./nix-direnv-gcroots.patch];
        });
      };
    };
  };
}
