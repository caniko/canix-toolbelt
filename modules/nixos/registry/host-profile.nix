# Read-only facts about the current host, derived from its
# canix-toolbelt.hosts entry so modules branch on one projection.
{
  config,
  lib,
  ...
}: let
  inherit (lib) mkOption types;
  cfg = config.canix-toolbelt.hostProfile;
  hostData =
    config.canix-toolbelt.hosts.${cfg.host}
    or (throw "canix-toolbelt.hostProfile: unknown host `${cfg.host}` in canix-toolbelt.hosts");
  fact = type: default: description:
    mkOption {
      inherit type default description;
      readOnly = true;
    };
in {
  imports = [./hosts.nix];

  options.canix-toolbelt.hostProfile = {
    host = mkOption {
      type = types.str;
      default = config.networking.hostName;
      defaultText = lib.literalExpression "config.networking.hostName";
      description = "canix-toolbelt.hosts entry describing this host.";
    };

    deviceType = fact (types.enum (import ../../../lib/deviceTypes.nix)) hostData.deviceType "Device type from canix-toolbelt.hosts.";
    isServer = fact types.bool (cfg.deviceType == "server") "Whether this host is a server.";
    isHeadless = fact types.bool (cfg.deviceType == "server") "True for hosts with no display. Disables browser-oriented and GUI features.";
    isAarch64 = fact types.bool (lib.hasPrefix "aarch64" config.nixpkgs.hostPlatform.system) "Whether this host runs on aarch64.";
    dataRoot = fact (types.nullOr types.str) hostData.dataRoot "Absolute path to this host's user-data filesystem root; null on hosts with no user data.";
    projectStateRoot = fact (types.nullOr types.str) hostData.projectStateRoot "Absolute path to mutable project state. An explicit host fact, never inferred from other roots.";
    flakeRoot = fact (types.nullOr types.str) hostData.flakeRoot "Absolute path to the host's canonical flake checkout.";
  };
}
