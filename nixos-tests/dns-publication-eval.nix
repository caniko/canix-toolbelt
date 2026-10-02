{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  stubs = {lib, ...}: {
    options =
      lib.genAttrs ["users" "systemd" "services"] (_:
        lib.mkOption {
          type = lib.types.attrs;
          default = {};
        })
      // {
        assertions = lib.mkOption {
          type = lib.types.listOf lib.types.attrs;
          default = [];
        };
        canix-toolbelt.services.httpSites = lib.mkOption {
          type = lib.types.attrsOf (lib.types.submodule {
            options =
              lib.genAttrs ["hostname" "access" "dnsPublication" "publicationTarget"] (_:
                lib.mkOption {type = lib.types.str;});
          });
          default = {};
        };
      };
  };
  site = {
    hostname = "media.example.test";
    access = "direct";
    dnsPublication = "managed";
    publicationTarget = "edge";
  };
  eval = extra:
    (lib.evalModules {
      modules = [
        stubs
        ../modules/nixos/services/dns-octodns-cloudflare.nix
        {
          _module.args = {inherit inputs pkgs;};
          canix-toolbelt.dns = {
            enable = true;
            zones."example.test" = {};
            publicationTargets.edge = {
              hostname = "edge.example.test";
              targetHost = "edge";
              ipv4 = "192.0.2.10";
            };
          };
          canix-toolbelt.services.httpSites.media = site;
        }
        extra
      ];
    }).config;
  valid = eval {};
  records = valid.canix-toolbelt.dns.dnsConfig.extraConfig.zones."example.test";
  apex = eval {canix-toolbelt.services.httpSites.media.hostname = lib.mkForce "example.test";};
  conflicts = eval {
    canix-toolbelt.dns.zones."example.test".exclude = [
      {
        name = "edge";
        type = "AAAA";
      }
    ];
  };
  limited = eval {canix-toolbelt.dns.zones."example.test".manageRecordTypes = ["A" "CNAME"];};
  failed = cfg: builtins.any (assertion: !assertion.assertion) cfg.assertions;
in
  mkEvalCheck {
    name = "dns-publication-eval";
    assertions = [
      {
        name = "dns-only-destination";
        assertion = records.edge.a.data == "192.0.2.10" && !records.edge.a.proxied && !(records.edge ? aaaa) && records.media.cname.data == "edge.example.test";
        message = "explicit targets must publish DNS-only IPv4 and redirect selected services";
      }
      {
        name = "apex-without-cname";
        assertion = apex.canix-toolbelt.dns.dnsConfig.extraConfig.zones."example.test"."".a.data == "192.0.2.10";
        message = "apex publication must emit addresses instead of a self CNAME";
      }
      {
        name = "single-writer";
        assertion = failed conflicts && failed limited;
        message = "excluded AAAA and partial address management must prevent publication";
      }
    ];
  }
