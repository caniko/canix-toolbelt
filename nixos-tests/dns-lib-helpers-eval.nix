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
    ];
  }
