{pkgs, ...}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  mkStubs = {lib, ...}: {
    options.age.secrets = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule ({name, ...}: {
        freeformType = lib.types.attrsOf lib.types.anything;
        options.path = lib.mkOption {
          type = lib.types.str;
          default = "/run/agenix/${name}";
          description = "Test stub for agenix's generated runtime secret path.";
        };
      }));
      default = {};
      description = "Test stub for agenix secrets.";
    };

    options.assertions = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      default = [];
      description = "Test stub for NixOS assertions.";
    };

    options.services.caddy = lib.mkOption {
      type = lib.types.submodule {
        options = {
          enable = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = "Test stub for services.caddy.enable.";
          };
          package = lib.mkOption {
            type = lib.types.package;
            default = pkgs.caddy;
            description = "Test stub for services.caddy.package.";
          };
          adapter = lib.mkOption {
            type = lib.types.str;
            default = "";
            description = "Test stub for services.caddy.adapter.";
          };
          configFile = lib.mkOption {
            type = lib.types.path;
            description = "Test stub for services.caddy.configFile.";
          };
          logDir = lib.mkOption {
            type = lib.types.str;
            default = "/var/log/caddy";
            description = "Test stub for services.caddy.logDir (referenced by generated log configs).";
          };
        };
      };
      default = {};
      description = "Test stub for services.caddy.";
    };

    options.systemd.services = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule {
        freeformType = lib.types.attrsOf lib.types.anything;
      });
      default = {};
      description = "Test stub for systemd services.";
    };

    options.networking = lib.mkOption {
      type = lib.types.submodule {
        freeformType = lib.types.attrsOf lib.types.anything;
        options = {
          hostName = lib.mkOption {
            type = lib.types.str;
            default = "thething";
          };
          firewall = lib.mkOption {
            type = lib.types.submodule {
              freeformType = lib.types.attrsOf lib.types.anything;
              options = {
                allowedTCPPorts = lib.mkOption {
                  type = lib.types.listOf lib.types.port;
                  default = [];
                };
                interfaces = lib.mkOption {
                  type = lib.types.attrsOf (lib.types.submodule {
                    freeformType = lib.types.attrsOf lib.types.anything;
                    options.allowedTCPPorts = lib.mkOption {
                      type = lib.types.listOf lib.types.port;
                      default = [];
                    };
                  });
                  default = {};
                };
              };
            };
            default = {};
            description = "Test stub for networking.firewall.";
          };
        };
      };
      default = {};
      description = "Test stub for networking.";
    };
  };

  evalFor = hostName:
    lib.evalModules {
      specialArgs = {inherit pkgs;};
      modules = [
        mkStubs
        ../modules/nixos/registry/hosts.nix
        ../modules/nixos/registry/services.nix
        ../modules/nixos/services/caddy-service-registry.nix
        {
          networking.hostName = hostName;
          canix-toolbelt.hosts.thething = {
            lanIp = "192.0.2.10";
            lanInterface = "lan0";
          };
          canix-toolbelt.hosts.atlas = {
            lanIp = "192.0.2.20";
            lanInterface = "enp1s0";
          };
          canix-toolbelt.services.caddy = {
            enable = true;
            useServiceRegistry = true;
            excludeVpnOnly = true;
            staticRoots.mta-sts = ../lib/caddy.nix;
            certificates = [
              {
                certificate = "/var/lib/acme/candee/fullchain.pem";
                key = "/var/lib/acme/candee/key.pem";
              }
            ];
          };
          canix-toolbelt.services.reverseProxyServices = [
            {
              name = "searxng";
              hostname = "search.example.com";
              port = 8888;
              targetHost = "thething";
              vpnOnly = true;
            }
            {
              name = "foundry";
              hostname = "vtt.example.com";
              port = 8030;
              targetHost = "atlas";
              lanExposed = true;
              routes = [
                {
                  paths = ["/api" "/api/*"];
                  targetHost = "atlas";
                  port = 8032;
                }
                {
                  targetHost = "atlas";
                  port = 8030;
                }
              ];
            }
            {
              name = "queryfabric";
              hostname = "query.example.com";
              port = 8780;
              targetHost = "thething";
              proxied = true;
            }
          ];
          canix-toolbelt.services.staticFileServices = [
            {
              name = "mta-sts";
              hostname = "mta-sts.example.com";
              staticRootName = "mta-sts";
            }
          ];
        }
      ];
    };

  eval = evalFor "thething";
  targetEval = evalFor "atlas";
  cfg = eval.config.canix-toolbelt.services.caddy;
  # readFile output embeds store paths (static roots), which fromJSON rejects;
  # assert on the raw JSON string instead.
  configJson = builtins.readFile eval.config.services.caddy.configFile;

  routesFor = hostname:
    lib.filter (route:
      lib.any (match: lib.elem hostname (match.host or [])) (route.match or []))
    cfg.routes;
  vttRoutes = routesFor "vtt.example.com";
  vttUpstream = route: lib.findFirst (handler: handler.handler or "" == "reverse_proxy") null (route.handle or []);
  mtaStsRoutes = routesFor "mta-sts.example.com";
in
  mkEvalCheck {
    name = "caddy-ingress-ha-eval";
    resultMessage = "Caddy ingress HA synthesis evaluated expected defaults";
    assertions = [
      {
        name = "certificates-loaded";
        assertion = lib.hasInfix ''"load_files":[{"certificate":"/var/lib/acme/candee/fullchain.pem","key":"/var/lib/acme/candee/key.pem"}]'' configJson;
        message = "expected certificates to be serialized into apps.tls.certificates.load_files";
      }
      {
        name = "vpn-only-route-excluded";
        assertion = routesFor "search.example.com" == [] && !(lib.hasInfix ''"search.example.com"'') configJson;
        message = "expected vpnOnly service routes to be excluded when excludeVpnOnly is set";
      }
      {
        name = "vpn-only-host-exemption-removed";
        assertion = !(lib.elem "search.example.com" cfg.cidrExemptHosts);
        message = "expected vpnOnly hostnames to be removed from the CIDR allowlist exemptions";
      }
      {
        name = "public-route-kept";
        assertion = builtins.length (routesFor "query.example.com") == 1;
        message = "expected non-vpn public service routes to be kept";
      }
      {
        name = "static-named-root";
        assertion = builtins.length mtaStsRoutes == 1 && lib.hasInfix ''"mta-sts.example.com"'' configJson;
        message = "expected named static root service to produce a route";
      }
      {
        name = "static-root-resolution";
        assertion = (lib.findFirst (handler: handler.handler or "" == "vars") null (builtins.head mtaStsRoutes).handle).root == ../lib/caddy.nix;
        message = "expected the static route vars root to resolve the named root";
      }
      {
        name = "foundry-route-count";
        assertion = builtins.length vttRoutes == 2;
        message = "expected both Foundry routes to remain";
      }
      {
        name = "foundry-api-backend";
        assertion = (vttUpstream (builtins.head vttRoutes)).upstreams == [{dial = "192.0.2.20:8032";}];
        message = "expected the API route to dial Atlas port 8032";
      }
      {
        name = "no-global-port-leak";
        assertion = !(lib.elem 8780 eval.config.networking.firewall.allowedTCPPorts);
        message = "expected non-lanExposed local service ports to stay out of the global firewall allowlist";
      }
      {
        name = "lan-exposed-per-interface";
        assertion = targetEval.config.networking.firewall.interfaces.enp1s0.allowedTCPPorts == [8032 8030];
        message = "expected lanExposed route backend ports on the target LAN interface only";
      }
    ];
  }
