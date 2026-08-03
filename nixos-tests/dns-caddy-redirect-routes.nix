{
  inputs,
  pkgs,
  ...
}: let
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  toolbeltLib = import ../lib {inherit (inputs.nixpkgs) lib;};
  directRoute = toolbeltLib.caddy.mkRedirectRoute {
    from = "old.example.com";
    to = "https://new.example.com";
    preservePath = false;
  };
  inherit
    ((inputs.fleetix.lib.projections.normalize {
      topology = {
        hosts = {};
        links = {};
        domains.redirects = [
          {
            from = "example.com";
            to = "https://www.example.com";
            status = 301;
            preservePath = true;
          }
        ];
        services = {};
      };
    }))
    domains
    ;

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
          inherit (domains) redirects;
        };

        system.stateVersion = "25.11";
      }
    ];
  };

  routes = moduleResult.config.canix-toolbelt.services.caddy.routes;
  redirectRoutes = builtins.filter (route:
    route.match or []
    != []
    && (builtins.head route.match).host or [] == ["example.com"])
  routes;
  redirectRoute =
    if redirectRoutes == []
    then null
    else builtins.head redirectRoutes;
in
  mkEvalCheck {
    name = "dns-caddy-redirect-routes";
    resultMessage = "DNS redirect intents projected to Caddy routes";
    assertions = [
      {
        name = "route-found";
        assertion = redirectRoute != null;
        message = "expected a Caddy route for example.com";
      }
      {
        name = "status-code";
        assertion = redirectRoute != null && (builtins.head redirectRoute.handle).status_code == 301;
        message = "expected redirect status code 301";
      }
      {
        name = "location-preserves-uri";
        assertion = redirectRoute != null && (builtins.head redirectRoute.handle).headers.Location == ["https://www.example.com{http.request.uri}"];
        message = "expected redirect Location to preserve request URI";
      }
      {
        name = "direct-helper-location";
        assertion = (builtins.head directRoute.handle).headers.Location == ["https://new.example.com"];
        message = "expected direct mkRedirectRoute call to omit request URI when preservePath=false";
      }
    ];
  }
