# Auto-wires agenix-rekey to all of the consuming flake's nixosConfigurations
# and (optionally) home-manager configurations. Removes the per-host boilerplate
# of listing every config manually under `agenix-rekey.nixosConfigurations`.
#
# Consumers still need to add `agenix-rekey.flakeModule` to their inputs/imports
# (this module does NOT import it for you, so you stay in control of pinning).
#
# Usage:
#
#   imports = [
#     inputs.agenix-rekey.flakeModule
#     inputs.canix-toolbelt.flakeModules.agenix-rekey-auto
#   ];
#
#   perSystem = _: {
#     canix-toolbelt.agenix-rekey-auto = {
#       enable = true;
#       # opt-out: include = name: !(lib.hasSuffix "-installer" name);
#     };
#   };
{
  flake-parts-lib,
  lib,
  self,
  ...
}: {
  options.perSystem = flake-parts-lib.mkPerSystemOption ({config, ...}: let
    cfg = config.canix-toolbelt.agenix-rekey-auto;
  in {
    options.canix-toolbelt.agenix-rekey-auto = {
      enable = lib.mkEnableOption "auto-wiring agenix-rekey to self.nixosConfigurations";

      include = lib.mkOption {
        type = lib.types.functionTo lib.types.bool;
        default = _: true;
        description = "Predicate `name: bool` filtering nixosConfigurations to include.";
      };

      collectHomeManagerConfigurations = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Forwarded to agenix-rekey.";
      };
    };

    config = lib.mkIf cfg.enable {
      agenix-rekey = {
        inherit (cfg) collectHomeManagerConfigurations;
        nixosConfigurations =
          lib.filterAttrs (name: _: cfg.include name) self.nixosConfigurations;
      };
    };
  });
}
