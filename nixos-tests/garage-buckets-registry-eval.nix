{pkgs}: let
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  evaluate = module:
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      system = "x86_64-linux";
      modules = [
        ../modules/nixos/services/garage-buckets-registry.nix
        module
      ];
    }).config;

  plain = evaluate {};

  seeded = evaluate {
    canix-toolbelt.garageBuckets.registry.sccache.seed = "test-seed";
  };

  withGarage = evaluate {
    imports = [../modules/nixos/services/garage.nix];
    canix-toolbelt.services.garage = {
      enable = true;
      package = pkgs.garage_2;
      buckets = ["sccache"];
    };
    canix-toolbelt.garageBuckets.registry.sccache.seed = "test-seed";
  };

  expected = let
    hash = builtins.hashString "sha256" "test-seed";
  in {
    accessKeyId = "GK" + builtins.substring 0 18 hash;
    secretAccessKey = builtins.substring 0 40 hash;
  };
in
  mkEvalCheck {
    name = "garage-buckets-registry-eval";
    resultMessage = "garage bucket registry derives credentials and provisions only on garage hosts";
    assertions = [
      {
        name = "credentials-derived-deterministically";
        assertion = seeded.canix-toolbelt.garageBuckets.credentials.sccache == expected;
        message = "registry credentials must be deterministic and match the reference derivation";
      }
      {
        name = "empty-registry-no-credentials";
        assertion = plain.canix-toolbelt.garageBuckets.credentials == {};
        message = "empty registry must expose no credentials";
      }
      {
        name = "provision-only-with-garage-service";
        assertion = plain.systemd.services ? garage-init-buckets == false;
        message = "no garage-init-buckets service without the garage service module";
      }
      {
        name = "provision-on-garage-host";
        assertion =
          withGarage.systemd.services.garage-init-buckets.serviceConfig.Type
          == "oneshot"
          && builtins.hasAttr "sccache" withGarage.canix-toolbelt.garageBuckets.credentials;
        message = "garage host must provision registered buckets through a oneshot";
      }
    ];
  }
