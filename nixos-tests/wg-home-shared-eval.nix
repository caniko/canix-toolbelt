{pkgs}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  evalWithLink = link:
    lib.evalModules {
      modules = [
        ../modules/nixos/registry/hosts.nix
        ../modules/nixos/networking/wg-home-shared.nix
        {canix-toolbelt.networking.links.wg-home = link;}
      ];
    };

  isSet = cfg: attr: (builtins.tryEval cfg.canix-toolbelt.networking.wgHome.${attr}).success;
  get = cfg: attr: (builtins.tryEval cfg.canix-toolbelt.networking.wgHome.${attr}).value or null;

  noLink = evalWithLink {};
  nullFallback = evalWithLink {
    cidr = "198.51.100.0/24";
    port = null;
    endpointHost = null;
    ddnsHost = "dyn.example.test";
    endpointSubdomain = "wg";
  };
  explicit = evalWithLink {
    cidr = "198.51.100.0/24";
    port = 1234;
    endpointHost = "wg.example.test";
  };
  subdomainOnly = evalWithLink {
    cidr = "198.51.100.0/24";
    endpointSubdomain = "mesh";
  };
in
  mkEvalCheck {
    name = "wg-home-shared-eval";
    resultMessage = "wg-home shared null-safe link derivation passed";
    assertions = [
      {
        name = "no-link-no-defaults";
        assertion = !(isSet noLink.config "endpointHost") && !(isSet noLink.config "port") && !(isSet noLink.config "vpnDomain");
        message = "without a declared link no wgHome defaults may be defined (and evaluation must not throw on {})";
      }
      {
        name = "null-skips-to-ddns";
        assertion = get nullFallback.config "endpointHost" == "dyn.example.test";
        message = "a declared-but-null endpointHost must fall through to ddnsHost";
      }
      {
        name = "null-port-falls-back";
        assertion = get nullFallback.config "port" == 54321;
        message = "a declared-but-null port must fall back to 54321";
      }
      {
        name = "vpn-domain-never-derived";
        assertion = !(isSet nullFallback.config "vpnDomain");
        message = "vpnDomain must never be derived from link fields; the consumer sets it explicitly";
      }
      {
        name = "explicit-values-kept";
        assertion = get explicit.config "endpointHost" == "wg.example.test" && get explicit.config "port" == 1234;
        message = "explicit link endpointHost and port must pass through unchanged";
      }
      {
        name = "subdomain-last-resort";
        assertion = get subdomainOnly.config "endpointHost" == "mesh";
        message = "endpointSubdomain is used only when endpointHost and ddnsHost are absent";
      }
    ];
  }
