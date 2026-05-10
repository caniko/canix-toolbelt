# Cross-arch shebang audit for NixOS closures.
#
# Walks `system.build.toplevel` of selected nixosConfigurations and verifies
# that every `#!/nix/store/...` interpreter is an ELF whose `e_machine` matches
# the host platform. Catches the classic cross-compilation footgun where a
# native-platform script (e.g. an aarch64 host's activation script) silently
# links a build-platform interpreter.
#
# Usage:
#
#   imports = [inputs.canix-toolbelt.flakeModules.shebang-audit];
#
#   perSystem = _: {
#     canix-toolbelt.shebang-audit = {
#       enable = true;
#       # Audit only nixosConfigurations whose name ends in "-crossbow":
#       filter = name: _: lib.hasSuffix "-crossbow" name;
#       # Or pick configurations explicitly:
#       # configurations = { thething-crossbow = self.nixosConfigurations.thething-crossbow; };
#     };
#   };
{
  lib,
  flake-parts-lib,
  self,
  ...
}: {
  options.perSystem = flake-parts-lib.mkPerSystemOption ({
    config,
    pkgs,
    ...
  }: let
    cfg = config.canix-toolbelt.shebang-audit;

    selected =
      if cfg.configurations != null
      then cfg.configurations
      else lib.filterAttrs cfg.filter self.nixosConfigurations;

    auditOne = name: nixosConfig: let
      hostSystem = nixosConfig.config.nixpkgs.hostPlatform.system;
      expected =
        cfg.archMap.${hostSystem}
        or (throw "canix-toolbelt.shebang-audit: no e_machine mapped for ${hostSystem}; extend canix-toolbelt.shebang-audit.archMap.");
    in
      pkgs.runCommand "shebang-audit-${name}" {} ''
              set -eu
              toplevel=${nixosConfig.config.system.build.toplevel}
              bad=""

              for path in $(${pkgs.nix}/bin/nix-store --query --requisites "$toplevel"); do
                [ -d "$path" ] || continue
                while IFS= read -r script; do
                  ${pkgs.coreutils}/bin/head -c 2 "$script" | ${pkgs.gnugrep}/bin/grep -q '^#!' || continue
                  interp=$(${pkgs.coreutils}/bin/head -c 256 "$script" 2>/dev/null \
                    | ${pkgs.gnused}/bin/sed -n '1{s|^#![[:space:]]*||;s|[[:space:]].*||;p}')
                  case "$interp" in
                    /nix/store/*) ;;
                    *) continue ;;
                  esac
                  [ -x "$interp" ] || continue
                  arch=$(${pkgs.coreutils}/bin/od -An -t x1 -j18 -N2 "$interp" \
                    | ${pkgs.coreutils}/bin/tr -d ' \n')
                  if [ "$arch" != "${expected}" ]; then
                    bad="$bad
        $script -> $interp (e_machine=$arch, expected=${expected})"
                  fi
                done < <(${pkgs.findutils}/bin/find "$path" -maxdepth 3 -type f \
                  -executable -not -name '*.so*' 2>/dev/null || true)
              done

              if [ -n "$bad" ]; then
                echo "shebang-audit: wrong-arch interpreters found in ${name} closure:" >&2
                echo "$bad" >&2
                exit 1
              fi

              touch "$out"
      '';
  in {
    options.canix-toolbelt.shebang-audit = {
      enable = lib.mkEnableOption "cross-arch shebang audit";

      filter = lib.mkOption {
        type = lib.types.functionTo (lib.types.functionTo lib.types.bool);
        default = _: _: true;
        description = ''
          Predicate `name: nixosConfig: bool` used to pick which
          `self.nixosConfigurations` to audit. Ignored if
          `configurations` is set.
        '';
      };

      configurations = lib.mkOption {
        type = lib.types.nullOr (lib.types.attrsOf lib.types.unspecified);
        default = null;
        description = ''
          Explicit `{ name = nixosConfig; }` attrset to audit. When set,
          `filter` is ignored.
        '';
      };

      archMap = lib.mkOption {
        type = lib.types.attrsOf lib.types.str;
        default = {
          "x86_64-linux" = "3e00";
          "aarch64-linux" = "b700";
          "i686-linux" = "0300";
          "riscv64-linux" = "f300";
          "armv7l-linux" = "2800";
        };
        description = ''
          Map from Nix system string to the little-endian hex of ELF
          `e_machine` (bytes 18-19). Extend for additional architectures.
        '';
      };
    };

    config = lib.mkIf cfg.enable {
      checks =
        lib.mapAttrs'
        (name: nixosConfig: {
          name = "shebang-audit-${name}";
          value = auditOne name nixosConfig;
        })
        selected;
    };
  });
}
