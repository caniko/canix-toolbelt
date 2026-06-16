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
    };
    modules = [
      ../modules/nixos/services/dns-octodns-cloudflare.nix
      {
        canix-toolbelt.dns = {
          enable = true;
          zones."example.com".records = [
            {
              name = "@";
              type = "CNAME";
              data = "target.example.net";
            }
          ];
        };

        system.stateVersion = "25.11";
      }
    ];
  };

  assertionMessages = builtins.map (assertion: assertion.message) moduleResult.config.assertions;
  hasExpectedAssertion =
    builtins.any
    (message:
      builtins.match ".*Apex.*CNAME.*RFC 1034.*Use type = \"ALIAS\".*" message
      != null)
    assertionMessages;
  dnsConfigEval =
    builtins.tryEval
    (builtins.deepSeq moduleResult.config.canix-toolbelt.dns.dnsConfig true);
in
  mkEvalCheck {
    name = "dns-apex-cname-assertion";
    resultMessage = "apex CNAME assertion failed as expected";
    assertions = [
      {
        name = "assertion-message-found";
        assertion = hasExpectedAssertion;
        message = "expected DNS apex CNAME assertion message to mention RFC 1034 and recommend ALIAS";
      }
      {
        name = "dns-config-failed";
        assertion = !dnsConfigEval.success;
        message = "expected DNS apex CNAME validation to fail when forcing dnsConfig";
      }
    ];
  }
