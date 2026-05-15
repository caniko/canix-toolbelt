# Generic host config profiles toggled at boot via NixOS specialisations.
#
# A profile is a named boolean state with a default value and zero or more
# per-specialisation overrides. The module synthesises one NixOS
# specialisation per name referenced across all profiles, so several profiles
# can contribute to the same specialisation. The current `enable` value is
# also mirrored into `home-manager.sharedModules` so user-level modules can
# query the same option.
{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.profiles;

  inherit
    (lib)
    attrNames
    concatLists
    filterAttrs
    genAttrs
    mapAttrs
    mapAttrs'
    mapAttrsToList
    mkDefault
    mkForce
    mkMerge
    mkOption
    nameValuePair
    types
    unique
    ;

  profileSubmodule = {config, ...}: {
    options = {
      description = mkOption {
        type = types.str;
        default = "";
        description = "Human-readable description of the profile.";
      };

      default = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Default value of `enable` when no specialisation targeting this
          profile is the booted one.
        '';
      };

      enable = mkOption {
        type = types.bool;
        description = ''
          Whether the profile is currently active. Defaults to `default`;
          overridden inside specialisations declared under `specialisations`.
        '';
      };

      specialisations = mkOption {
        type = types.attrsOf (types.submodule {
          options.enable = mkOption {
            type = types.bool;
            description = "Value of the profile's `enable` while this specialisation is booted.";
          };
        });
        default = {};
        description = ''
          Map of NixOS specialisation name to the profile's `enable` value
          while that specialisation is the booted configuration. Multiple
          profiles may target the same specialisation.
        '';
      };
    };

    config.enable = mkDefault config.default;
  };

  specNames =
    unique (concatLists (mapAttrsToList (_: p: attrNames p.specialisations) cfg));

  profilesForSpec = specName:
    filterAttrs (_: p: p.specialisations ? ${specName}) cfg;
in {
  options.canix-toolbelt.profiles = mkOption {
    type = types.attrsOf (types.submodule profileSubmodule);
    default = {};
    description = "Named host config profiles toggled at boot via specialisations.";
  };

  config = mkMerge [
    # Marker files so tooling (e.g. `canix travel status`) can distinguish a
    # host that declares the profile from one that doesn't. Contents are
    # exactly `true\n` or `false\n` so a plain read is unambiguous.
    {
      environment.etc = mapAttrs' (pName: p:
        nameValuePair "canix-profiles/${pName}.enable" {
          text = "${lib.boolToString p.enable}\n";
        })
      cfg;
    }

    {
      specialisation = genAttrs specNames (specName: {
        configuration.canix-toolbelt.profiles =
          mapAttrs' (pName: p:
            nameValuePair pName {
              enable = mkForce p.specialisations.${specName}.enable;
            })
          (profilesForSpec specName);
      });
    }

    {
      home-manager.sharedModules = [
        ({lib, ...}: {
          options.canix-toolbelt.profiles = lib.mkOption {
            type = lib.types.attrsOf (lib.types.submodule {
              options.enable = lib.mkOption {
                type = lib.types.bool;
                default = false;
                description = "Mirror of the NixOS-level profile `enable` state.";
              };
            });
            default = {};
            description = "Profile state mirrored from the NixOS configuration.";
          };
        })
        {
          canix-toolbelt.profiles =
            mapAttrs (_: p: {enable = p.enable;}) cfg;
        }
      ];
    }
  ];
}
