{
  inputs,
  pkgs,
  ...
}: let
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  moduleResult = inputs.nixpkgs.lib.nixosSystem {
    inherit (pkgs.stdenv.hostPlatform) system;
    specialArgs = {
      inherit inputs;
      crossbowBuildPkgs = pkgs;
    };
    modules = [
      ../modules/nixos/services/dns-octodns-cloudflare.nix
      {
        canix-toolbelt.dns = {
          enable = true;
          cloudflareToken.secretPath = "/run/secrets/cloudflare-token";
          reconciler.applyForce = true;
          zones."example.com".records = [
            {
              name = "www";
              type = "A";
              data = "192.0.2.1";
            }
          ];
        };

        system.stateVersion = "25.11";
      }
    ];
  };

  services = moduleResult.config.systemd.services;
  planExec = services.cloudflare-octodns.serviceConfig.ExecStart;
  applyExec = services.cloudflare-octodns-apply.serviceConfig.ExecStart;
in
  mkEvalCheck {
    name = "dns-octodns-apply-force";
    resultMessage = "octoDNS apply force option is scoped to apply service";
    assertions = [
      {
        name = "dry-run-has-no-force";
        assertion = builtins.match ".*--force.*" planExec == null;
        message = "cloudflare-octodns dry-run service must not receive --force";
      }
      {
        name = "apply-has-doit";
        assertion = builtins.match ".* --doit.*" applyExec != null;
        message = "cloudflare-octodns-apply must still pass --doit";
      }
      {
        name = "apply-has-force";
        assertion = builtins.match ".* --force.*" applyExec != null;
        message = "cloudflare-octodns-apply must pass --force when reconciler.applyForce is true";
      }
    ];
  }
