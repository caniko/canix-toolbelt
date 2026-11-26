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
  alertFiles = ["/run/agenix/gatus-ntfy.env" "/run/agenix/gatus-smtp.env"];
  alerts = [{type = "ntfy";}];
  emailAlerts = [{type = "email";}];
  endpoint = {
    name = "manual";
    url = "http://127.0.0.1:8092/health";
    conditions = ["[STATUS] == 200"];
  };
  providers = {
    ntfy = {
      token = "\${GATUS_NTFY_TOKEN}";
      url = "https://ntfy.example.test";
      topic = "health";
      default-alert = {
        enabled = true;
        send-on-resolved = true;
      };
    };
    email = {
      from = "gatus@example.test";
      username = "gatus@example.test";
      password = "\${GATUS_SMTP_PASS}";
      host = "smtp.example.test";
      port = 587;
      to = "oncall@example.test";
    };
  };
  withPrivate = options:
    evaluate (lib.recursiveUpdate externalModule {services.gatusInstances.private = options;});
  alerting = withPrivate {
    environmentFiles = alertFiles;
    alerting = providers;
    defaultAlerts = alerts;
    endpoints = [
      (endpoint // {alerts = emailAlerts;})
      (endpoint
        // {
          name = "opt-out";
          alerts = [];
        })
    ];
  };
  alertingSettings = alerting.services.gatusInstances.private.settings;
  networkByName = lib.listToAttrs (map (e: lib.nameValuePair e.name e) alertingSettings.endpoints);
  nullAlerts = {endpoints = [(endpoint // {alerts = null;})];};
  nullWithDefaults = withPrivate (nullAlerts
    // {
      alerting = providers;
      defaultAlerts = alerts;
    });
  extraFilesOnly = withPrivate {
    environmentFile = null;
    environmentFiles = ["/run/agenix/health.env"];
  };
  passes = result: lib.all (a: a.assertion) result.assertions;
  rejects = message: result: map (a: a.message) (lib.filter (a: !a.assertion) result.assertions) == [message];
  missingProvider = "gatusInstances.private: every enabled alert must reference a non-empty provider attrset in alerting.";
  invalidType = "gatusInstances.private: every alert must have a non-empty string type.";
in
  assert disabled.systemd.services == {};
  assert lib.all (a: a.assertion) cfg.assertions;
  assert !(lib.all (a: a.assertion) bad.assertions);
  assert cfg.services.gatusInstances.public.settings.ui.header == "Public";
  assert cfg.services.gatusInstances.public.settings.web.address == "127.0.0.1";
  assert (builtins.head cfg.services.gatusInstances.private.settings.endpoints).url == "http://127.0.0.1:8092/health";
  assert (builtins.head cfg.services.gatusInstances.public.settings.endpoints).ui.hide-errors;
  assert cfg.services.gatusInstances.public.settings.storage.path != cfg.services.gatusInstances.private.settings.storage.path;
  assert cfg.services.gatusInstances.public.settings.storage
  == {
    type = "sqlite";
    path = "/var/lib/gatus-public/gatus.db";
  };
  assert (evaluate (lib.recursiveUpdate module {
    services.gatusInstances.public.storage = {
      type = "postgres";
      path = "postgres://gatus@/gatus?host=/run/postgresql";
    };
  })).services.gatusInstances.public.settings.storage
  == {
    type = "postgres";
    path = "postgres://gatus@/gatus?host=/run/postgresql";
  };
  assert !(lib.all (a: a.assertion) (evaluate (lib.recursiveUpdate module {services.gatusInstances.public.storage.type = "postgres";})).assertions);
  assert (evaluate (lib.recursiveUpdate module {services.gatusInstances.public.storage.type = "memory";})).services.gatusInstances.public.settings.storage == {type = "memory";};
  assert lib.all (a: a.assertion) external.assertions;
  assert !(lib.all (a: a.assertion) externalOnly.assertions);
  assert (builtins.head external.services.gatusInstances.private.settings.external-endpoints).token == "\${GATUS_EXTERNAL_TOKEN}";
  assert external.systemd.services.gatus-private.serviceConfig.EnvironmentFile == "/run/agenix/health.env";
  assert external.systemd.services.gatus-private.serviceConfig.LoadCredential == ["ca-0.crt:/runtime/ca.crt"];
  assert external.systemd.services.gatus-private.environment.SSL_CERT_DIR == "%d";
  assert (builtins.head external.services.gatusInstances.private.settings.endpoints).client.dns-resolver == "udp://192.0.2.1:53";
  assert !(cfg.services.gatusInstances.public.settings ? alerting);
  assert !((builtins.head cfg.services.gatusInstances.public.settings.endpoints) ? alerts);
  assert !((builtins.head external.services.gatusInstances.private.settings.external-endpoints) ? alerts);
  assert passes alerting;
  assert alertingSettings.alerting == providers;
  assert alerting.systemd.services.gatus-private.serviceConfig.EnvironmentFile == ["/run/agenix/health.env"] ++ alertFiles;
  assert passes extraFilesOnly;
  assert extraFilesOnly.systemd.services.gatus-private.serviceConfig.EnvironmentFile == ["/run/agenix/health.env"];
  assert (builtins.head (lib.filter (e: e.name != "manual" && e.name != "opt-out") alertingSettings.endpoints)).alerts == alerts;
  assert (builtins.head alertingSettings.external-endpoints).alerts == alerts;
  assert networkByName.manual.alerts == emailAlerts;
  assert networkByName.opt-out.alerts == [];
  assert alerting.services.gatusInstances.public.settings == cfg.services.gatusInstances.public.settings;
  assert !(alerting.systemd.services.gatus-public.serviceConfig ? EnvironmentFile);
  assert passes (withPrivate nullAlerts);
  assert passes nullWithDefaults;
  assert (lib.findFirst (e: e.name == endpoint.name) {} nullWithDefaults.services.gatusInstances.private.settings.endpoints).alerts == null;
  assert rejects "gatusInstances.private: endpoint alerts must be a list or null." (withPrivate {endpoints = [(endpoint // {alerts = "ntfy";})];});
  assert rejects missingProvider (withPrivate {defaultAlerts = alerts;});
  assert rejects missingProvider (withPrivate {endpoints = [(endpoint // {inherit alerts;})];});
  assert rejects missingProvider (withPrivate {
    alerting.ntfy = {};
    defaultAlerts = alerts;
  });
  assert lib.all (provider: !(builtins.tryEval (withPrivate {alerting.ntfy = provider;}).services.gatusInstances.private.alerting.ntfy).success) [null "invalid"];
  assert rejects invalidType (withPrivate {defaultAlerts = [{}];});
  assert rejects invalidType (withPrivate {endpoints = [(endpoint // {alerts = [{type = "";}];})];});
  assert rejects invalidType (withPrivate {endpoints = [(endpoint // {alerts = [{type = 42;}];})];});
  assert passes (withPrivate {
    defaultAlerts = [
      {
        type = "ntfy";
        enabled = false;
      }
    ];
  });
    if fixture
    then {
      settings = external.services.gatusInstances.private.settings;
      inherit ((builtins.head external.services.gatusInstances.private.externalChecks)) key;
    }
    else true
