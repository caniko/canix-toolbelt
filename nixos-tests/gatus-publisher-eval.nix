{pkgs}: let
  inherit (pkgs) lib;
  evaluate = module:
    (lib.evalModules {
      specialArgs = {inherit pkgs;};
      modules = [
        ../modules/nixos/services/gatus-health-publisher.nix
        ../modules/nixos/services/gatus-external-ingress.nix
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
            systemd.timers = lib.mkOption {
              type = lib.types.attrs;
              default = {};
            };
            networking.hostName = lib.mkOption {
              type = lib.types.str;
              default = "hub";
            };
            networking.firewall.extraInputRules = lib.mkOption {
              type = lib.types.lines;
              default = "";
            };
            services.caddy = lib.mkOption {
              type = lib.types.attrs;
              default = {};
            };
            services.gatusInstances = lib.mkOption {
              type = lib.types.attrs;
              default = {};
            };
          };
          config.services.caddy.logDir = "/var/log/caddy";
        })
        module
      ];
    }).config;
  check = host: {
    key = "infra_${host}";
    source = {
      intervalSeconds = 60;
      timeoutSeconds = 10;
      probe = {
        type = "unit";
        inherit host;
        unit = "worker.service";
      };
    };
  };
  settings = {
    services.gatusHealthPublisher = {
      enable = true;
      command = ["/bin/test-publisher"];
      url = "http://192.0.2.1:8093";
      credentialEnvFile = "/run/agenix/health.env";
      checks = [(check "hub") (check "other")];
    };
    services.gatusInstances.staff = {
      enable = true;
      port = 8092;
      environmentFile = "/run/agenix/health.env";
      externalChecks = [(check "hub")];
    };
    services.gatusExternalIngress = {
      enable = true;
      instance = "staff";
      address = "192.0.2.1";
      port = 8093;
      interface = "wg-private";
      peers = ["192.0.2.2"];
    };
  };
  cfg = evaluate settings;
  tooMany = evaluate (lib.recursiveUpdate settings {services.gatusHealthPublisher.checks = lib.genList (_: check "hub") 100;});
  emptyHost = evaluate (lib.recursiveUpdate settings {networking.hostName = "unused";});
  ingress = (builtins.fromJSON cfg.services.caddy.configFile.text).apps.http.servers.gatus-health;
in
  assert lib.all (a: a.assertion) cfg.assertions;
  assert !(lib.all (a: a.assertion) tooMany.assertions);
  assert !(emptyHost.systemd.services ? gatus-health-publisher);
  assert cfg.systemd.services.gatus-health-publisher.serviceConfig.DynamicUser;
  assert cfg.systemd.services.gatus-health-publisher.serviceConfig.LoadCredential == ["health.env:/run/agenix/health.env"];
  assert cfg.systemd.services.gatus-health-publisher.serviceConfig.TimeoutStartSec == "26s";
  assert (builtins.head (builtins.head ingress.routes).match).method == ["POST"];
  assert (builtins.head (builtins.head ingress.routes).match).remote_ip.ranges == ["192.0.2.2"];
  assert ingress.listen == ["192.0.2.1:8093"];
  assert ingress.automatic_https.disable;
  assert lib.hasInfix "ip daddr 192.0.2.1" cfg.networking.firewall.extraInputRules; {inherit ingress;}
