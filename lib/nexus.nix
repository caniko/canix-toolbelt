# Pure helpers for the nexus toggle interface.
#
# Consumers feed in a per-host toggle attrset and receive a NixOS
# module attrset that drives `canix-toolbelt.profiles.<name>` for
# that host. Host-agnostic: no host names, no consumer paths.
{lib}: let
  inherit (lib) mkOption types;

  toggleSubmodule = _: {
    options = {
      description = mkOption {
        type = types.str;
        description = "Human-readable description of the toggle.";
      };

      default = mkOption {
        type = types.bool;
        description = "Default value for the generated profile.";
      };

      deviceTypes = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Allowed device types for this toggle. Empty means any.";
      };

      specialisations = mkOption {
        type = types.attrsOf (types.submodule {
          options.enable = mkOption {
            type = types.bool;
            description = "Value for the generated profile in this specialisation.";
          };
        });
        default = {};
        description = "Specialisations generated for the resulting profile.";
      };
    };

    config.deviceTypes = lib.mkDefault [];
    config.specialisations = lib.mkDefault {};
  };

  assertToggle = {
    hostName,
    profileName,
    toggle,
    deviceType,
  }: let
    allowed = toggle.deviceTypes or [];
    ok = allowed == [] || builtins.elem deviceType allowed;
  in
    lib.assertMsg ok ''
      canix-toolbelt.lib.nexus: toggle `${profileName}` is restricted to
      deviceTypes = ${builtins.toJSON allowed}, but host `${hostName}` has
      deviceType = "${deviceType}". Either drop the deviceTypes constraint
      on `${profileName}` or remove the toggle from `${hostName}`.
    '';

  mkProfilesForHost = {
    hostName,
    hostToggles ? {},
    deviceType,
  }: {
    canix-toolbelt.profiles = lib.mapAttrs (profileName: toggle:
      assert assertToggle {inherit hostName profileName toggle deviceType;}; {
        inherit (toggle) description default;
        specialisations = toggle.specialisations or {};
      })
    hostToggles;
  };
in {
  inherit assertToggle mkProfilesForHost toggleSubmodule;
}
