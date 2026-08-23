# Opinionated "ops" devShell for NixOS fleets using agenix-rekey + Attic.
#
# Bundles the tooling needed to administer a multi-host NixOS flake:
#   - agenix-rekey CLI (rekey secrets to host pubkeys)
#   - attic-client    (push/pull binary cache artefacts)
#   - rage + age-plugin-fido2-hmac (decrypt/encrypt age secrets)
#   - hardware introspection: pciutils, lshw, nvme-cli, gptfdisk, efibootmgr,
#     libfido2, mesa-demos
#   - ripgrep
#
# Exposes the result as `devShells.${name}` (default `ops`) and sets
# `AGENIX_REKEY_ADD_TO_GIT=true` so rekeyed secrets land staged.
#
# Usage:
#
#   imports = [inputs.canix-toolbelt.flakeModules.ops-shell];
#
#   perSystem = {config, pkgs, ...}: {
#     canix-toolbelt.ops-shell = {
#       enable = true;
#       agenixRekeyPackage = config.agenix-rekey.package;
#       extraPackages = [pkgs.uv];
#       welcome = "canix ops shell";
#     };
#   };
{
  lib,
  flake-parts-lib,
  ...
}: {
  options.perSystem = flake-parts-lib.mkPerSystemOption ({
    config,
    pkgs,
    ...
  }: let
    cfg = config.canix-toolbelt.ops-shell;

    baseTools = import ../lib/opsShellPackages.nix pkgs;

    welcomeHook = lib.optionalString (cfg.welcome != null) ''
      echo ${lib.escapeShellArg cfg.welcome}
    '';
  in {
    options.canix-toolbelt.ops-shell = {
      enable = lib.mkEnableOption "ops devShell";

      name = lib.mkOption {
        type = lib.types.str;
        default = "ops";
        description = "Attribute name under `devShells`.";
      };

      asDefault = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "If true, also expose this shell as `devShells.default`.";
      };

      agenixRekeyPackage = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = null;
        description = ''
          The agenix-rekey CLI. Typically `config.agenix-rekey.package`. Set
          to null to omit (e.g. when the flake doesn't import agenix-rekey).
        '';
      };

      extraPackages = lib.mkOption {
        type = lib.types.listOf lib.types.package;
        default = [];
        description = "Extra packages to add to the shell.";
      };

      extraShellHook = lib.mkOption {
        type = lib.types.lines;
        default = "";
        description = "Appended to the shellHook.";
      };

      extraEnv = lib.mkOption {
        type = lib.types.attrsOf lib.types.str;
        default = {};
        description = "Extra environment variables exported in the shell.";
      };

      welcome = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Optional banner echoed when the shell starts.";
      };

      mkShell = lib.mkOption {
        type = lib.types.functionTo lib.types.package;
        default = pkgs.mkShell;
        defaultText = "pkgs.mkShell";
        description = ''
          Shell builder. Override to use a custom builder (e.g.
          `harbor-rs.lib.mkDevShell` configured for Rust cross-compilation).
          Receives `{packages, shellHook, ...attrs}` where `attrs` includes
          every entry of `extraEnv`.
        '';
      };
    };

    config = lib.mkIf cfg.enable {
      devShells = let
        shell = cfg.mkShell ({
            packages =
              baseTools
              ++ lib.optional (cfg.agenixRekeyPackage != null) cfg.agenixRekeyPackage
              ++ cfg.extraPackages;

            shellHook = welcomeHook + cfg.extraShellHook;

            AGENIX_REKEY_ADD_TO_GIT = true;
          }
          // cfg.extraEnv);
      in
        {${cfg.name} = shell;}
        // lib.optionalAttrs cfg.asDefault {default = shell;};
    };
  });
}
