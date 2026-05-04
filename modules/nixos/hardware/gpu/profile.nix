# Shared GPU profile option. Vendor-specific modules read this to gate
# kernel param / driver tweaks tied to a particular card model.
{lib, ...}: {
  options.canix-toolbelt.hardware.gpu.profile = lib.mkOption {
    type = lib.types.nullOr lib.types.str;
    default = null;
    example = "intel-arc-a770-xe";
    description = ''
      Identifier for a specific GPU model profile. Vendor modules use this to
      apply model-specific tweaks (force-probe IDs, driver selection, etc.).
      Set to null when no model-specific overrides are needed.
    '';
  };
}
