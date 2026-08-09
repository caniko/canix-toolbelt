# Deterministic garage bucket registry.
#
# Each bucket maps a git-anchored seed to Garage S3 access credentials
# (access key id + secret). The credential derivation is the single
# implementation of the "GK" key math shared by every fleet client;
# consumers read the read-only `credentials` export instead of
# reimplementing it. The garage service module provisions the buckets
# of the host running Garage; this module is data only.
{
  config,
  lib,
  ...
}: let
  inherit (lib) mkOption types;
  cfg = config.canix-toolbelt.garageBuckets;
in {
  options.canix-toolbelt.garageBuckets = {
    registry = mkOption {
      type = types.attrsOf (types.submodule {
        options.seed = mkOption {
          type = types.str;
          description = "Git-anchored seed deriving the bucket's deterministic S3 access key pair.";
        };
      });
      default = {};
      description = "Bucket name -> provisioning metadata. The attr name is the bucket name.";
    };

    credentials = mkOption {
      type = types.attrsOf (types.attrsOf types.str);
      readOnly = true;
      default =
        lib.mapAttrs (_: bucket: let
          hash = builtins.hashString "sha256" bucket.seed;
        in {
          accessKeyId = "GK" + builtins.substring 0 18 hash;
          secretAccessKey = builtins.substring 0 40 hash;
        })
        cfg.registry;
      description = "Derived S3 access key pairs per registered bucket.";
    };
  };
}
