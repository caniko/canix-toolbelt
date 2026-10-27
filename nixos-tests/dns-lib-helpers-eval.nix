{pkgs}: let
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;
  inherit (pkgs) lib;
  inherit ((import ../lib {inherit lib;})) dns;
  dynamicHosts = [
    {
      fqdn = "example.com";
      proxied = true;
    }
    {
      fqdn = "mail.example.com";
      proxied = false;
    }
    {
      fqdn = "other.example.net";
      proxied = true;
    }
  ];
in
  mkEvalCheck {
    name = "dns-lib-helpers-eval";
    resultMessage = "DNS library helper eval assertions passed";
    assertions = [
      {
        name = "dynamic-host-excludes";
        assertion =
          dns.dynamicHostExcludes {
            inherit lib dynamicHosts;
            zone = "example.com";
          }
          == [
            {
              name = "@";
              type = "A";
            }
            {
              name = "@";
              type = "AAAA";
            }
            {
              name = "mail";
              type = "A";
            }
            {
              name = "mail";
              type = "AAAA";
            }
          ];
        message = "expected dynamic hosts inside a zone to become A/AAAA excludes";
      }
      {
        name = "proxied-expression";
        assertion =
          dns.cloudflareDdnsProxiedExpression {inherit dynamicHosts;}
          == "is(example.com) || is(other.example.net)";
        message = "expected proxied dynamic hosts to render in source order";
      }
      {
        name = "proxied-expression-empty";
        assertion =
          dns.cloudflareDdnsProxiedExpression {
            dynamicHosts = [
              {
                fqdn = "mail.example.com";
                proxied = false;
              }
            ];
          }
          == "false";
        message = "expected no proxied dynamic hosts to render as false";
      }
      {
        name = "ddns-exclude-drift-clean";
        assertion =
          dns.ddnsExcludeDrift {
            inherit lib;
            zones."example.com".exclude = dns.dynamicHostExcludes {
              inherit lib dynamicHosts;
              zone = "example.com";
            };
            ip4Domains = ["example.com" "mail.example.com"];
            ip6Domains = ["example.com" "mail.example.com"];
          }
          == {
            missingExcludes = [];
            orphanExcludes = [];
          };
        message = "expected generated dynamic-host excludes to cover their DDNS records";
      }
      {
        name = "ddns-exclude-drift";
        assertion =
          dns.ddnsExcludeDrift {
            inherit lib;
            zones = {
              "example.com".exclude = [
                {
                  name = "@";
                  type = "A";
                }
                {
                  name = "stale";
                  type = "AAAA";
                }
                {
                  name = "txt";
                  type = "TXT";
                }
              ];
              "sub.example.com".exclude = [
                {
                  name = "host";
                  type = "A";
                }
              ];
            };
            ip4Domains = ["example.com" "host.sub.example.com" "other.example.net"];
            ip6Domains = ["example.com"];
          }
          == {
            missingExcludes = [
              {
                fqdn = "other.example.net";
                type = "A";
                reason = "no managed zone owns this FQDN";
              }
              {
                fqdn = "example.com";
                type = "AAAA";
                zone = "example.com";
                reason = "missing AAAA exclude";
              }
            ];
            orphanExcludes = [
              {
                name = "stale";
                type = "AAAA";
                zoneName = "example.com";
              }
            ];
          };
        message = "expected missing, unowned and orphan DDNS excludes to be reported with the most specific zone";
      }
      {
        name = "ddns-exclude-drift-zone-scope";
        assertion =
          dns.ddnsExcludeDrift {
            inherit lib;
            zones = {
              # The parent-zone exclude cannot protect a child-zone record.
              "example.com".exclude = [
                {
                  name = "host.sub";
                  type = "A";
                }
              ];
              "Sub.Example.com".exclude = [
                {
                  name = "HOST";
                  type = "A";
                }
              ];
              # octoDNS leaves A records of this zone alone.
              "example.org".manageRecordTypes = ["TXT"];
            };
            ip4Domains = ["host.sub.EXAMPLE.com" "example.org"];
          }
          == {
            missingExcludes = [];
            orphanExcludes = [
              {
                name = "host.sub";
                type = "A";
                zoneName = "example.com";
              }
            ];
          };
        message = "expected zone-scoped, case-insensitive matching that honours manageRecordTypes";
      }
    ];
  }
