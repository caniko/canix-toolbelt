{
  inputs,
  pkgs,
  ...
}: let
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
  pkgs.runCommand "dns-apex-cname-assertion" {
    assertionMessageFound =
      if hasExpectedAssertion
      then "1"
      else throw "expected DNS apex CNAME assertion message to mention RFC 1034 and recommend ALIAS";
    dnsConfigFailed =
      if dnsConfigEval.success
      then throw "expected DNS apex CNAME validation to fail when forcing dnsConfig"
      else "1";
  } ''
    mkdir -p "$out"
    printf '%s\n' "apex CNAME assertion failed as expected" > "$out/result"
  ''
