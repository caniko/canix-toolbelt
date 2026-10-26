{
  inputs,
  pkgs,
  ...
}: let
  inherit (pkgs) lib;
  inherit (import ./lib/eval-checks.nix {inherit pkgs;}) mkEvalCheck;

  topology = {
    schemaVersion = 2;
    links.wg-home = {
      subnet = "198.51.100.0/24";
      port = 51820;
      endpointSubdomain = "wg";
    };
    hosts = {
      edge1 = {
        deviceType = "server";
        network = {
          lanIp = "192.0.2.11";
          lanInterface = "edge0";
        };
        links = {};
      };
      edge2 = {
        deviceType = "server";
        network = {
          lanIp = "192.0.2.12";
          lanInterface = "edge0";
        };
        links = {};
      };
      backend = {
        deviceType = "server";
        network = {
          lanIp = "192.0.2.30";
          lanInterface = "lan0";
        };
        links = {};
      };
      hub = {
        deviceType = "server";
        network = {
          lanIp = "192.0.2.40";
          lanInterface = "lan0";
        };
        links.wg-home = {
          address = "198.51.100.1";
          role = "server";
          publicKey = "hub-key";
        };
      };
    };
    domains = {
      zones = ["example.test"];
      managedZones = ["example.test"];
    };
    services = {
      endpoints = {
        local = {
          targetHost = "edge1";
          port = 8000;
          transport = "http";
          bind = "loopback";
        };
        app = {
          targetHost = "backend";
          port = 9000;
          transport = "https";
          bind = "loopback";
          remoteVia = "app-relay";
          tlsServerName = "app.internal";
        };
        app-relay = {
          targetHost = "backend";
          port = 9100;
          transport = "http";
          bind = "lan";
        };
        stream = {
          targetHost = "backend";
          port = 9200;
          transport = "h2c";
          bind = "lan";
        };
        private-cache = {
          targetHost = "hub";
          port = 9300;
          transport = "http";
          bind = "vpn";
        };
        failover-primary = {
          targetHost = "backend";
          port = 9400;
          transport = "http";
          bind = "loopback";
          remoteVia = "failover-relay";
        };
        failover-relay = {
          targetHost = "backend";
          port = 9410;
          transport = "http";
          bind = "lan";
        };
        failover-lan = {
          targetHost = "hub";
          port = 9420;
          transport = "http";
          bind = "lan";
        };
      };
      httpSites = {
        app = {
          hostname = "app.example.test";
          ingress = "public";
          access = "cloudflare";
          dnsPublication = "managed";
          routes = [
            {
              match = {
                paths = [
                  {
                    type = "exact";
                    value = "/api";
                  }
                ];
                absentQueryParams = [];
              };
              action = {
                type = "proxy";
                endpoint = "app";
                stripPrefix = "/api";
              };
              authPolicy = "users";
              responseHeaders.X-Service = ["app"];
            }
            {
              match = {
                paths = [
                  {
                    type = "prefix";
                    value = "/stream";
                  }
                ];
                absentQueryParams = [];
              };
              action = {
                type = "proxy";
                endpoint = "stream";
              };
            }
            {
              match = {
                paths = [
                  {
                    type = "exact";
                    value = "/login";
                  }
                ];
                absentQueryParams = ["return"];
              };
              action = {
                type = "redirect";
                to = "https://auth.example.test/start";
                status = 302;
                preserveUri = false;
              };
            }
            {
              match = {
                paths = [];
                absentQueryParams = [];
              };
              action = {
                type = "respond";
                status = 404;
                body = "missing";
              };
              responseHeaders.Cache-Control = ["no-store"];
            }
          ];
        };
        direct = {
          hostname = "direct.example.test";
          ingress = "public";
          access = "direct";
          dnsPublication = "external";
          routes = [
            {
              match = {
                paths = [];
                absentQueryParams = [];
              };
              action = {
                type = "respond";
                status = 200;
                body = "direct";
              };
            }
          ];
        };
        files = {
          hostname = "files.example.test";
          ingress = "solo";
          access = "direct";
          dnsPublication = "managed";
          routes = [
            {
              match = {
                paths = [];
                absentQueryParams = [];
              };
              action = {
                type = "files";
                rootRef = "docs";
                indexNames = ["index.htm" "index.html"];
              };
            }
          ];
        };
        local = {
          hostname = "local.example.test";
          ingress = "solo";
          access = "direct";
          dnsPublication = "none";
          routes = [
            {
              match = {
                paths = [];
                absentQueryParams = [];
              };
              action = {
                type = "proxy";
                endpoint = "local";
              };
            }
          ];
        };
        vpn = {
          hostname = "private.example.test";
          ingress = "vpn";
          access = "vpn";
          dnsPublication = "none";
          routes = [
            {
              match = {
                paths = [];
                absentQueryParams = [];
              };
              action = {
                type = "respond";
                status = 200;
                body = "vpn";
              };
            }
          ];
        };
        failover = {
          hostname = "failover.example.test";
          ingress = "public";
          access = "cloudflare";
          dnsPublication = "none";
          routes = [
            {
              match = {
                paths = [];
                absentQueryParams = [];
              };
              action = {
                type = "proxy";
                endpoint = "failover-primary";
                fallbackEndpoints = ["failover-lan"];
              };
            }
          ];
        };
        lan = {
          hostname = "lan.example.test";
          ingress = "lan";
          access = "lan";
          dnsPublication = "none";
          routes = [
            {
              match = {
                paths = [];
                absentQueryParams = [];
              };
              action = {
                type = "respond";
                status = 200;
                body = "lan";
              };
            }
          ];
        };
      };
    };
    deployment.ingressGroups = {
      public = {
        scope = "public";
        hosts = ["edge1" "edge2"];
      };
      solo = {
        scope = "public";
        hosts = ["edge1"];
      };
      vpn = {
        scope = "vpn";
        hosts = ["hub"];
      };
      lan = {
        scope = "lan";
        hosts = ["edge2"];
      };
    };
  };

  stubs = {lib, ...}: {
    options.assertions = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      default = [];
    };
    options.services.caddy = lib.mkOption {
      type = lib.types.submodule {
        freeformType = lib.types.attrsOf lib.types.anything;
        options = {
          package = lib.mkOption {
            type = lib.types.package;
            default = pkgs.caddy;
          };
          configFile = lib.mkOption {type = lib.types.path;};
          logDir = lib.mkOption {
            type = lib.types.str;
            default = "/var/log/caddy";
          };
        };
      };
      default = {};
    };
    options.services.dnsmasq = lib.mkOption {
      type = lib.types.submodule {freeformType = lib.types.attrsOf lib.types.anything;};
      default = {};
    };
    options.systemd.services = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule {freeformType = lib.types.attrsOf lib.types.anything;});
      default = {};
    };
    options.networking = lib.mkOption {
      type = lib.types.submodule {
        freeformType = lib.types.attrsOf lib.types.anything;
        options = {
          hostName = lib.mkOption {type = lib.types.str;};
          firewall = lib.mkOption {
            type = lib.types.submodule {
              freeformType = lib.types.attrsOf lib.types.anything;
              options = {
                allowedTCPPorts = lib.mkOption {
                  type = lib.types.listOf lib.types.port;
                  default = [];
                };
                extraInputRules = lib.mkOption {
                  type = lib.types.lines;
                  default = "";
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
          };
        };
      };
      default = {};
    };
  };

  evalFor = hostName: overrides:
    lib.evalModules {
      specialArgs = {inherit inputs pkgs;};
      modules = [
        stubs
        ../modules/nixos/registry/hosts.nix
        ../modules/nixos/registry/services.nix
        ../modules/nixos/services/caddy-service-registry.nix
        ../modules/nixos/networking/vpn-dns.nix
        {
          networking.hostName = hostName;
          canix-toolbelt.fleetix = {
            enable = true;
            topology = lib.recursiveUpdate topology overrides;
          };
          canix-toolbelt.services.caddy = {
            enable = true;
            ingressListeners = {
              public = ["192.0.2.240:443"];
              solo = ["192.0.2.241:443"];
              vpn = ["198.51.100.1:443"];
              lan = ["192.0.2.12:443"];
            };
            cloudflareCidrs = ["203.0.113.0/24"];
            staticRoots.docs = ../lib;
            authProviders.users = {
              driver = "generic";
              client_id = "users";
            };
            certificates = [
              {
                certificate = "/var/lib/acme/example/fullchain.pem";
                key = "/var/lib/acme/example/key.pem";
              }
            ];
          };
          canix-toolbelt.services.ingressGroups.public.sourceIps = ["192.0.2.240"];
          canix-toolbelt.networking.vpn-dns = {
            enable = hostName == "hub";
            domain = "example.test";
          };
        }
      ];
    };

  edge1 = evalFor "edge1" {};
  edge2 = evalFor "edge2" {};
  backend = evalFor "backend" {};
  hub = evalFor "hub" {};
  invalid = overrides: message:
    builtins.any (a: !a.assertion && lib.hasPrefix message a.message) (evalFor "edge1" overrides).config.assertions;
  failoverRoutes = overrides: [
    {
      match = {
        paths = [];
        absentQueryParams = [];
      };
      action =
        {
          type = "proxy";
          endpoint = "failover-primary";
        }
        // overrides;
    }
  ];
  edge1Servers = edge1.config.canix-toolbelt.services.caddy.servers;
  publicRoutes = edge1Servers.public.routes;
  routeHosts = routes: lib.concatMap (route: (builtins.head route.match).host or []) routes;
  appRoutes = builtins.filter (route: (builtins.head route.match).host == ["app.example.test"]) publicRoutes;
  exactRoute = builtins.elemAt appRoutes 0;
  prefixRoute = builtins.elemAt appRoutes 1;
  redirectRoute = builtins.elemAt appRoutes 2;
  fallbackRoute = builtins.elemAt appRoutes 3;
  handler = name: route: lib.findFirst (item: item.handler or "" == name) null route.handle;
  relayRoute = builtins.head backend.config.canix-toolbelt.services.caddy.servers.relay-app-relay.routes;
  dnsAddresses = hub.config.services.dnsmasq.settings.address;
  failoverProxy = handler "reverse_proxy" (lib.findFirst (route: (builtins.head route.match).host == ["failover.example.test"]) null publicRoutes);
in
  mkEvalCheck {
    name = "service-topology-v2-eval";
    resultMessage = "Fleetix service topology v2 registry evaluated expected Caddy, DNS, and firewall behavior";
    nativeBuildInputs = [pkgs.jq];
    runtimeScript = ''
      jq -e '.apps.tls.certificates.load_files == [{"certificate":"/var/lib/acme/example/fullchain.pem","key":"/var/lib/acme/example/key.pem"}]' ${edge1.config.services.caddy.configFile} >/dev/null
    '';
    assertions = [
      {
        name = "fleetix-populates-v2-registry";
        assertion = edge1.config.canix-toolbelt.services.endpoints.app.port == 9000 && hub.config.canix-toolbelt.services.endpoints.private-cache.bind == "vpn" && edge1.config.canix-toolbelt.services.ingressGroups.public.scope == "public";
        message = "expected Fleetix v2 endpoints and ingress groups to populate the registry";
      }
      {
        name = "vpn-endpoint-not-lan-exposed";
        assertion = !(builtins.elem 9300 (hub.config.networking.firewall.interfaces.lan0.allowedTCPPorts or []));
        message = "VPN-bound endpoint ports must not be exposed on the LAN firewall";
      }
      {
        name = "public-vpn-disjoint";
        assertion = builtins.hasAttr "public" edge1Servers && !(builtins.hasAttr "vpn" edge1Servers) && builtins.hasAttr "vpn" hub.config.canix-toolbelt.services.caddy.servers && !(builtins.hasAttr "public" hub.config.canix-toolbelt.services.caddy.servers) && !(builtins.elem "private.example.test" (routeHosts publicRoutes)) && !(builtins.elem "app.example.test" (routeHosts hub.config.canix-toolbelt.services.caddy.servers.vpn.routes));
        message = "public and VPN sites must render on disjoint current-host servers";
      }
      {
        name = "current-host-membership";
        assertion = builtins.hasAttr "solo" edge1Servers && !(builtins.hasAttr "solo" edge2.config.canix-toolbelt.services.caddy.servers);
        message = "an ingress server must render only on declared group hosts";
      }
      {
        name = "public-access-gate";
        assertion = edge1Servers.public.cidrAllowlist == ["203.0.113.0/24"] && edge1Servers.public.cidrExemptHosts == ["direct.example.test"] && hub.config.canix-toolbelt.services.caddy.servers.vpn.cidrAllowlist == [];
        message = "public Cloudflare sites need an edge gate, direct hosts an exemption, and VPN no Cloudflare gate";
      }
      {
        name = "remote-via-dial";
        assertion = (handler "reverse_proxy" exactRoute).upstreams == [{dial = "192.0.2.30:9100";}];
        message = "remote loopback endpoints must dial their LAN relay";
      }
      {
        name = "local-loopback-dial";
        assertion = (handler "reverse_proxy" (builtins.elemAt edge1Servers.solo.routes 1)).upstreams == [{dial = "127.0.0.1:8000";}];
        message = "same-host loopback endpoints must dial 127.0.0.1";
      }
      {
        name = "relay-generation";
        assertion = backend.config.canix-toolbelt.services.caddy.servers.relay-app-relay.listen == ["192.0.2.30:9100"] && (handler "reverse_proxy" relayRoute).upstreams == [{dial = "127.0.0.1:9000";}] && (handler "reverse_proxy" relayRoute).transport.tls.server_name == "app.internal" && !backend.config.canix-toolbelt.services.caddy.servers.relay-app-relay.automaticHttps && edge1Servers.public.automaticHttps;
        message = "the endpoint target must serve a host-matched relay to its loopback source, with HTTPS disabled on the relay but enabled on ingress servers";
      }
      {
        name = "h2c-transport";
        assertion = (handler "reverse_proxy" prefixRoute).transport.versions == ["h2c" "2"];
        message = "h2c endpoints must request Caddy h2c and HTTP/2 transport versions";
      }
      {
        name = "ordered-paths-and-fallback";
        assertion = (builtins.head exactRoute.match).path == ["/api"] && (builtins.head prefixRoute.match).path == ["/stream*"] && !(builtins.hasAttr "path" (builtins.head fallbackRoute.match));
        message = "exact, prefix, and empty fallback routes must retain source order and semantics";
      }
      {
        name = "absent-query-redirect";
        assertion = (builtins.head redirectRoute.match).not == [{query.return = ["*"];}] && (handler "static_response" redirectRoute).headers.Location == ["https://auth.example.test/start"];
        message = "redirect routes must render absent-query matching and preserveUri=false";
      }
      {
        name = "files-root-and-indexes";
        assertion = (handler "vars" (builtins.head edge1Servers.solo.routes)).root == ../lib && (handler "file_server" (builtins.head edge1Servers.solo.routes)).index_names == ["index.htm" "index.html"];
        message = "files actions must resolve host-local roots and ordered index names";
      }
      {
        name = "auth-and-response-headers";
        assertion = (handler "authenticator" exactRoute).portal_name == "users" && (handler "headers" exactRoute).response.set.X-Service == ["app"] && (handler "headers" fallbackRoute).response.set.Cache-Control == ["no-store"];
        message = "auth policies and route response headers must render";
      }
      {
        name = "vpn-dns-from-access";
        assertion = builtins.elem "/private.example.test/198.51.100.1" dnsAddresses && !(builtins.elem "/app.example.test/198.51.100.1" dnsAddresses);
        message = "VPN DNS must derive records from site access, not hostname suffix";
      }
      {
        name = "no-global-backend-firewall-leak";
        assertion = backend.config.networking.firewall.allowedTCPPorts == [] && !(lib.elem 9100 backend.config.networking.firewall.interfaces.lan0.allowedTCPPorts);
        message = "backend and relay ports must never enter the global firewall allowlist";
      }
      {
        name = "lan-endpoint-interface-scope";
        assertion = backend.config.networking.firewall.interfaces.lan0.allowedTCPPorts == [9200];
        message = "ordinary LAN endpoints must open only on the target LAN interface";
      }
      {
        name = "relay-firewall-source-scope";
        assertion = lib.hasInfix ''iifname "lan0"'' backend.config.networking.firewall.extraInputRules && lib.hasInfix "ip saddr { 192.0.2.11, 192.0.2.12, 192.0.2.240 }" backend.config.networking.firewall.extraInputRules && lib.hasInfix "tcp dport 9100 accept" backend.config.networking.firewall.extraInputRules;
        message = "relay firewall access must be interface- and ingress-source-scoped, including failover VIPs";
      }
      {
        name = "single-endpoint-proxy-unchanged";
        assertion = builtins.toJSON (handler "reverse_proxy" (builtins.elemAt edge1Servers.solo.routes 1)) == ''{"handler":"reverse_proxy","upstreams":[{"dial":"127.0.0.1:8000"}]}'' && !(handler "reverse_proxy" exactRoute ? load_balancing) && !(handler "reverse_proxy" exactRoute ? health_checks);
        message = "single-endpoint proxy routes must render exactly as before, without failover policy";
      }
      {
        name = "ordered-failover-upstreams";
        assertion = failoverProxy.upstreams == [{dial = "192.0.2.30:9410";} {dial = "192.0.2.40:9420";}] && failoverProxy.load_balancing.selection_policy.policy == "first" && failoverProxy.health_checks.passive.max_fails == 1 && !(failoverProxy ? transport);
        message = "fallback endpoints must render as ordered upstreams with strict first-available failover";
      }
      {
        name = "failover-primary-relay-and-fallback-lan-port";
        assertion = backend.config.canix-toolbelt.services.caddy.servers.relay-failover-relay.listen == ["192.0.2.30:9410"] && lib.hasInfix "tcp dport 9410 accept" backend.config.networking.firewall.extraInputRules && hub.config.networking.firewall.interfaces.lan0.allowedTCPPorts == [9420];
        message = "a remote loopback primary needs its relay and a LAN fallback its LAN port";
      }
      {
        name = "lan-ingress-no-cidr-gate";
        assertion = edge2.config.canix-toolbelt.services.caddy.servers.lan.cidrAllowlist == [] && edge2.config.canix-toolbelt.services.caddy.servers.lan.cidrExemptHosts == [] && routeHosts edge2.config.canix-toolbelt.services.caddy.servers.lan.routes == ["lan.example.test"] && !(builtins.hasAttr "lan" edge1Servers);
        message = "LAN ingress groups must render on their hosts without a Cloudflare CIDR gate";
      }
      {
        name = "vpn-dns-skips-lan-sites";
        assertion = !(builtins.any (lib.hasPrefix "/lan.example.test/") dnsAddresses);
        message = "VPN DNS must not publish LAN-only sites";
      }
      {
        name = "lan-site-on-public-group-rejected";
        assertion = invalid {services.httpSites.lan.ingress = "public";} "canix-toolbelt Caddy registry: site `lan` must reference an ingress group whose scope matches";
        message = "a LAN site on a public ingress group must fail evaluation";
      }
      {
        name = "duplicate-fallback-rejected";
        assertion = invalid {services.httpSites.failover.routes = failoverRoutes {fallbackEndpoints = ["failover-lan" "failover-lan"];};} "canix-toolbelt service-registry: site `failover` fallbackEndpoints must be unique" && invalid {services.httpSites.failover.routes = failoverRoutes {fallbackEndpoints = ["failover-primary"];};} "canix-toolbelt service-registry: site `failover` fallbackEndpoints must be unique";
        message = "duplicate fallbacks and fallbacks equal to the primary must fail evaluation";
      }
      {
        name = "fallback-transport-mismatch-rejected";
        assertion = invalid {services.endpoints.failover-lan.transport = "h2c";} "canix-toolbelt Caddy registry: site `failover` proxy endpoints must share transport";
        message = "fallbacks dialed with a different transport must fail evaluation";
      }
      {
        name = "fallback-default-sni-matches-explicit-hostname";
        assertion = let
          cfg = evalFor "edge1" {
            services.endpoints = {
              failover-lan.transport = "https";
              failover-lan-tls = {
                targetHost = "hub";
                port = 9421;
                transport = "https";
                bind = "lan";
                tlsServerName = "failover.example.test";
              };
            };
            services.httpSites.failover.routes = failoverRoutes {
              endpoint = "failover-lan";
              fallbackEndpoints = ["failover-lan-tls"];
            };
          };
        in
          builtins.all (a: a.assertion || !lib.hasPrefix "canix-toolbelt Caddy registry: site `failover` proxy endpoints must share" a.message) cfg.config.assertions;
        message = "an unset tlsServerName must compare as the site hostname it sends";
      }
      {
        name = "fallback-sharing-a-relay-rejected";
        assertion = invalid {
          services.endpoints.failover-twin = {
            targetHost = "backend";
            port = 9401;
            transport = "http";
            bind = "loopback";
            remoteVia = "failover-relay";
          };
          services.httpSites.failover.routes = failoverRoutes {fallbackEndpoints = ["failover-twin"];};
        } "canix-toolbelt Caddy registry: site `failover` proxy endpoints must resolve to distinct dial addresses";
        message = "a fallback behind the primary's relay is unreachable and must fail evaluation";
      }
      {
        name = "valid-topology-assertions-pass";
        assertion = builtins.all (a: a.assertion) (edge1.config.assertions ++ edge2.config.assertions ++ backend.config.assertions ++ hub.config.assertions);
        message = "the fixture topology must satisfy every registry assertion";
      }
    ];
  }
