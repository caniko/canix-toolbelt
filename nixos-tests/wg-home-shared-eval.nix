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

  evalWithoutLink = lib.evalModules {
    modules = [
      ../modules/nixos/registry/hosts.nix
      ../modules/nixos/networking/wg-home-shared.nix
    ];
  };

  # Option metadata: distinguishes "intentionally undefined" from an
  # evaluation failure (tryEval success cannot tell those apart).
  isDefined = ev: attr: ev.options.canix-toolbelt.networking.wgHome.${attr}.isDefined or false;
  get = ev: attr: ev.config.canix-toolbelt.networking.wgHome.${attr};

  noLink = evalWithLink {};
  absentLink = evalWithoutLink;
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
        assertion = !(isDefined noLink "endpointHost") && !(isDefined noLink "port") && !(isDefined noLink "vpnDomain");
        message = "with an empty declared link no wgHome defaults may be defined (and evaluation must not throw on {})";
      }
      {
        name = "absent-link-no-defaults";
        assertion = !(isDefined absentLink "endpointHost") && !(isDefined absentLink "port") && !(isDefined absentLink "vpnDomain");
        message = "with no wg-home link declared at all no wgHome defaults may be defined";
      }
      {
        name = "null-skips-to-ddns";
        assertion = (isDefined nullFallback "endpointHost") && get nullFallback "endpointHost" == "dyn.example.test";
        message = "a declared-but-null endpointHost must fall through to ddnsHost";
      }
      {
        name = "null-port-falls-back";
        assertion = get nullFallback "port" == 54321;
        message = "a declared-but-null port must fall back to 54321";
      }
      {
        name = "vpn-domain-never-derived";
        assertion = !(isDefined nullFallback "vpnDomain");
        message = "vpnDomain must never be derived from link fields; the consumer sets it explicitly";
      }
      {
        name = "explicit-values-kept";
        assertion = get explicit "endpointHost" == "wg.example.test" && get explicit "port" == 1234;
        message = "explicit link endpointHost and port must pass through unchanged";
      }
      {
        name = "subdomain-last-resort";
        assertion = get subdomainOnly "endpointHost" == "mesh";
        message = "endpointSubdomain is used only when endpointHost and ddnsHost are absent";
      }
    ];
  }
