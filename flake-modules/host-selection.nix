# Shared flake-level host availability policy.
#
# This controls whether host-scoped flake outputs are emitted. It deliberately
# does not remove a host from Fleetix topology, SSH inventory, or runtime
# reachability.
{lib, ...}: {
  options.canix-toolbelt.host-selection = lib.mkOption {
    type = lib.types.attrsOf (lib.types.submodule {
      options.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether host-scoped flake outputs are emitted.";
      };
    });
    default = {};
    description = ''
      Flake-level host availability policy. Hosts omitted from this attrset
      remain enabled. This does not change the Fleetix host inventory.
    '';
  };
}
