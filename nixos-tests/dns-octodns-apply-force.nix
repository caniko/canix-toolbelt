{
  inputs,
  pkgs,
  ...
}: let
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  fakeBuildPkgs = {
    stdenv.hostPlatform.system = "marker-build-system";
    inherit (pkgs) runCommand;
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
          cloudflareToken.secretPath = "/run/secrets/cloudflare-token";
          reconciler.applyForce = true;
          pagesZone = "example.com";
          pagesSites = [
            {
              subdomain = "docs";
              repository = "caniko/docs";
              cnameTarget = "caniko.github.io";
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

  services = moduleResult.config.systemd.services;
  pagesRecord = moduleResult.config.canix-toolbelt.dns.dnsConfig.extraConfig.zones."example.com".docs.cname;
  planExec = services.cloudflare-octodns.serviceConfig.ExecStart;
  applyExec = services.cloudflare-octodns-apply.serviceConfig.ExecStart;

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
  collisionResult = inputs.nixpkgs.lib.nixosSystem {
    inherit (pkgs.stdenv.hostPlatform) system;
    specialArgs = {
      inputs = fakeInputs;
      crossbowBuildPkgs = pkgs;
      canixCrossPackage = _name: package: package;
    };
    modules = [
      ../modules/nixos/services/dns-octodns-cloudflare.nix
      {
        canix-toolbelt.dns = {
          enable = true;
          pagesZone = "example.com";
          pagesSites = [
            {
              subdomain = "docs";
              repository = "caniko/docs";
              cnameTarget = "caniko.github.io";
            }
          ];
          zones."example.com".records = [
            {
              name = "Docs";
              type = "A";
              data = "192.0.2.1";
            }
          ];
        };
        system.stateVersion = "25.11";
      }
    ];
  };
  collisionEvaluation = builtins.tryEval (
    builtins.deepSeq collisionResult.config.canix-toolbelt.dns.dnsConfig true
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
        name = "github-pages-cname-is-synthesized";
        assertion = pagesRecord.data == "caniko.github.io";
        message = "Pages topology must synthesize the configured provider CNAME target";
      }
      {
        name = "cname-collisions-fail-evaluation";
        assertion = !collisionEvaluation.success;
        message = "CNAME records must not coexist with another record type at the same name";
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
    ];
  }
