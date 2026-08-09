{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  evaluate = buckets:
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      system = "x86_64-linux";
      modules = [
        ../modules/nixos/services/garage-buckets-registry.nix
        ../modules/nixos/services/garage.nix
        {
          system.stateVersion = "25.11";
          canix-toolbelt = {
            services.garage = {
              enable = true;
              package = pkgs.garage_2;
              buckets = builtins.attrNames buckets;
              settings = {
                metadata_dir = "/tmp/garage/meta";
                data_dir = "/tmp/garage/data";
                rpc_secret = "x";
                replication_factor = 1;
              };
            };
            garageBuckets.registry = buckets;
          };
        }
      ];
    }).config;

  seeded = evaluate {sccache = {seed = "rs-harbor-sccache-garage-2026";};};
  none = evaluate {};
in
  mkEvalCheck {
    name = "garage-buckets-registry-eval";
    resultMessage = "garage bucket registry derives credentials and provisions buckets";
    assertions = [
      {
        name = "credentials-derived";
        assertion =
          seeded.canix-toolbelt.garageBuckets.credentials.sccache
          == {
            accessKeyId = "GK96c3cf18ad59bf4aff";
            secretAccessKey = "96c3cf18ad59bf4aff52c3d2adaaf5058a374111";
          };
        message = "sccache registry entry must derive the fleet-known access key pair";
      }
      {
        name = "buckets-unit-created";
        assertion = lib.hasAttrByPath ["systemd" "services" "garage-init-buckets"] seeded;
        message = "declared buckets must generate the provisioning unit";
      }
      {
        name = "no-buckets-no-unit";
        assertion = !(builtins.hasAttr "garage-init-buckets" none.systemd.services);
        message = "empty registry must not generate the provisioning unit";
      }
      {
        name = "init-unit-is-oneshot";
        assertion =
          seeded.systemd.services.garage-init-buckets.serviceConfig.Type
          == "oneshot";
        message = "provisioning unit must be a oneshot";
      }
    ];
  }
