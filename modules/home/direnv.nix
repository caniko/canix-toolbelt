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
          # resholve installs from its original store input, not the unpacked
          # tree. Patch the installed script after resolution so the change
          # survives that copy, preserving the base package's patches/hooks.
          postFixup =
            (old.postFixup or "")
            + ''
              chmod u+w "$out/share/nix-direnv/direnvrc"
              ${lib.getExe pkgs.patch} --directory="$out" --strip=1 < ${./nix-direnv-gcroots.patch}
              chmod u-w "$out/share/nix-direnv/direnvrc"
            '';
        });
      };
    };
  };
}
