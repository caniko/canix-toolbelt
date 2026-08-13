{
  inputs,
  pkgs,
  ...
}: let
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  fakeBuildPkgs = {
    stdenv.hostPlatform.system = "marker-build-system";
    runCommand = pkgs.runCommand;
  };
  fakeInputs =
    inputs
    // {
      dns-manager = {
        lib.generate = dnsPkgs: {
          cloudflare = _:
            dnsPkgs.runCommand "dns-manager-build-pkgs-${dnsPkgs.stdenv.hostPlatform.system}" {} ''
              mkdir -p "$out"
            '';
          caddyRoutes = _: throw "dnsGenerate.caddyRoutes must not be called by canix-toolbelt.dns";
        };
      };
      secret-manager.packages.${pkgs.stdenv.hostPlatform.system}.default =
        pkgs.writeShellScriptBin "secret-manager" "exit 1";
    };

  moduleResult = inputs.nixpkgs.lib.nixosSystem {
    inherit (pkgs.stdenv.hostPlatform) system;
    specialArgs = {
      inherit inputs;
      crossbowBuildPkgs = pkgs;
      canixCrossPackage = _name: package: package;
    };
    modules = [
      ../modules/nixos/services/dns-octodns-cloudflare.nix
      {
        canix-toolbelt.dns = {
          enable = true;
          autoSynthesizeCodebergPagesCnames = false;
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

  localOnlyResult = inputs.nixpkgs.lib.nixosSystem {
    inherit (pkgs.stdenv.hostPlatform) system;
    specialArgs = {
      inherit inputs;
      crossbowBuildPkgs = pkgs;
      canixCrossPackage = _name: package: package;
    };
    modules = [
      ../modules/nixos/services/dns-octodns-cloudflare.nix
      {
        canix-toolbelt.dns = {
          enable = true;
          autoSynthesizeCodebergPagesCnames = false;
          reconciler.enable = false;
          cloudflareToken.secretPath = "/run/secrets/cloudflare-token";
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

  optimizationResult = inputs.nixpkgs.lib.nixosSystem {
    inherit (pkgs.stdenv.hostPlatform) system;
    specialArgs = {
      inputs = fakeInputs;
      crossbowBuildPkgs = pkgs;
      canixCrossPackage = _name: package: package;
    };
    modules = [
      ../modules/nixos/services/dns-octodns-cloudflare.nix
      {
        _module.args.dnsManagerBuildPkgs = fakeBuildPkgs;

        canix-toolbelt.dns = {
          enable = true;
          autoSynthesizeCodebergPagesCnames = false;
          redirects = [
            {
              from = "example.com";
              to = "https://www.example.com";
            }
          ];
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

  optimizationOctodnsConfig = toString optimizationResult.config.canix-toolbelt.dns.octodnsConfig;
  optimizationRedirectRoute = builtins.head (
    builtins.filter
    (route: route.match or [] != [] && (builtins.head route.match).host or [] == ["example.com"])
    optimizationResult.config.canix-toolbelt.services.caddy.routes
  );
in
  mkEvalCheck {
    name = "dns-octodns-apply-force";
    resultMessage = "octoDNS apply force option and dns-manager build-pkgs optimization are stable";
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
      {
        name = "dns-manager-uses-build-pkgs";
        assertion = builtins.match ".*dns-manager-build-pkgs-marker-build-system.*" optimizationOctodnsConfig != null;
        message = "dns-manager render derivations must use dnsManagerBuildPkgs when provided";
      }
      {
        name = "redirect-routes-avoid-dns-manager-renderer";
        assertion = (builtins.head optimizationRedirectRoute.handle).headers.Location == ["https://www.example.com{http.request.uri}"];
        message = "Caddy redirect routes must be rendered locally without dnsGenerate.caddyRoutes";
      }
      {
        name = "local-only-has-no-runtime-services";
        assertion = !(localOnlyResult.config.systemd.services ? cloudflare-octodns) && !(localOnlyResult.config.systemd.services ? cloudflare-octodns-apply);
        message = "reconciler.enable = false must keep octoDNS services out of the host configuration";
      }
      {
        name = "local-only-has-no-runtime-user";
        assertion = !(localOnlyResult.config.users.users ? cloudflare-octodns);
        message = "reconciler.enable = false must keep the octoDNS user out of the host configuration";
      }
    ];
  }
