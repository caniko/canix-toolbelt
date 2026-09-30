{
  pkgs,
  fleetixLib,
  fixture ? false,
}: let
  inherit (pkgs) lib;
  evaluate = module:
    (lib.evalModules {
      specialArgs = {
        inherit pkgs;
        inputs.fleetix.lib = fleetixLib;
      };
      modules = [
        ../modules/nixos/services/gatus-instances.nix
        ({lib, ...}: {
          options = {
            assertions = lib.mkOption {
              type = lib.types.listOf lib.types.attrs;
              default = [];
            };
            systemd.services = lib.mkOption {
              type = lib.types.attrs;
              default = {};
            };
            networking.hostName = lib.mkOption {
              type = lib.types.str;
              default = "hub";
            };
          };
        })
        module
      ];
    }).config;
  topology.services = {
    endpoints.monitor = {
      targetHost = "hub";
      port = 8092;
      bind = "loopback";
      transport = "http";
    };
    catalog.monitor = {
      displayName = "Monitor Process";
      category = "Infrastructure";
      endpoints = ["monitor"];
      health.health.probe = {
        type = "http";
        endpoint = "monitor";
        path = "/health";
      };
    };
    httpSites.web = {
      hostname = "web.example.test";
      access = "direct";
      ingress = "edge";
    };
    catalog.web = {
      displayName = "Website";
      category = "Applications";
      visibility = "public";
      sites = ["web"];
      health.health.probe = {
        type = "http";
        site = "web";
      };
    };
  };
  module = {
    services.gatusDiscovery = {
      topology =
        topology
        // {
          hosts.hub = {};
          deployment.ingressGroups.edge.scope = "public";
        };
      domains = ["example.test" "private.test"];
      defaultDomain = "private.test";
    };
    services.gatusInstances = {
      public = {
        enable = true;
        port = 8091;
        title = "Public";
        selection.domain = "example.test";
      };
      private = {
        enable = true;
        port = 8092;
        title = "Private";
        selection = {
          domain = "private.test";
          includeInternal = true;
        };
      };
    };
  };
  cfg = evaluate module;
  disabled = evaluate {};
  bad = evaluate (lib.recursiveUpdate module {services.gatusInstances.private.port = 8091;});
  externalModule = lib.recursiveUpdate module {
    services.gatusDiscovery.topology.services.catalog.worker = {
      displayName = "Worker";
      category = "Infrastructure";
      health.health.probe = {
        type = "unit";
        host = "hub";
        unit = "worker.service";
      };
    };
    services.gatusInstances.private.environmentFile = "/run/agenix/health.env";
    services.gatusInstances.private.client.dns-resolver = "udp://192.0.2.1:53";
    services.gatusInstances.private.trustedCertificateFiles = ["/runtime/ca.crt"];
  };
  external = evaluate externalModule;
  externalOnly = evaluate (lib.recursiveUpdate externalModule {
    services.gatusInstances.private.endpoints = lib.mkForce [];
  });
in
  assert disabled.systemd.services == {};
  assert lib.all (a: a.assertion) cfg.assertions;
  assert !(lib.all (a: a.assertion) bad.assertions);
  assert cfg.services.gatusInstances.public.settings.ui.header == "Public";
  assert cfg.services.gatusInstances.public.settings.web.address == "127.0.0.1";
  assert (builtins.head cfg.services.gatusInstances.private.settings.endpoints).url == "http://127.0.0.1:8092/health";
  assert (builtins.head cfg.services.gatusInstances.public.settings.endpoints).ui.hide-errors;
  assert cfg.services.gatusInstances.public.settings.storage.path != cfg.services.gatusInstances.private.settings.storage.path;
  assert lib.all (a: a.assertion) external.assertions;
  assert !(lib.all (a: a.assertion) externalOnly.assertions);
  assert (builtins.head external.services.gatusInstances.private.settings.external-endpoints).token == "\${GATUS_EXTERNAL_TOKEN}";
  assert external.systemd.services.gatus-private.serviceConfig.EnvironmentFile == "/run/agenix/health.env";
  assert external.systemd.services.gatus-private.serviceConfig.LoadCredential == ["ca-0.crt:/runtime/ca.crt"];
  assert external.systemd.services.gatus-private.environment.SSL_CERT_DIR == "%d";
  assert (builtins.head external.services.gatusInstances.private.settings.endpoints).client.dns-resolver == "udp://192.0.2.1:53";
    if fixture
    then {
      settings = external.services.gatusInstances.private.settings;
      inherit ((builtins.head external.services.gatusInstances.private.externalChecks)) key;
    }
    else true
