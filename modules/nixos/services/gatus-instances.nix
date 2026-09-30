# Independent native Gatus instances with optional Fleetix service discovery.
{
  config,
  lib,
  pkgs,
  inputs ? {},
  ...
}: let
  inherit (lib) mkOption types;
  cfg = config.services.gatusInstances;
  discovery = config.services.gatusDiscovery;
  fleetix = inputs.fleetix.lib or (throw "gatusDiscovery requires inputs.fleetix.lib");
  enabled = lib.filterAttrs (_: instance: instance.enable) cfg;
  discovered = lib.filterAttrs (_: instance: instance.selection != null) enabled;
  inventories = lib.mapAttrs (_: instance:
    fleetix.gatus.inventory {
      inherit (discovery) topology domains defaultDomain;
      hostName = config.networking.hostName;
      inherit (instance.selection) domain includeInternal;
    })
  discovered;
  coverage = fleetix.gatus.coverage {inherit (discovery) topology;};
  expectedChecks = lib.concatMap (service: let
    profile = discovery.topology.services.catalog.${service};
  in
    lib.optionals ((profile.lifecycle or "active") == "active")
    (map (id: "${service}/${id}") (builtins.attrNames (profile.health or {}))))
  (builtins.attrNames (discovery.topology.services.catalog or {}));
  selectedChecks = lib.concatMap (inventory: map (check: "${check.service}/${check.id}") inventory.monitors) (builtins.attrValues inventories);
  button = types.submodule {
    options = {
      name = mkOption {
        type = types.str;
        description = "Button text.";
      };
      link = mkOption {
        type = types.str;
        description = "Button URL.";
      };
    };
  };
  selection = types.submodule {
    options = {
      domain = mkOption {
        type = types.str;
        description = "Service domain selected by this dashboard.";
      };
      includeInternal = mkOption {
        type = types.bool;
        default = false;
        description = "Include restricted service checks; requires an authenticated frontend.";
      };
    };
  };
  instanceType = types.submodule ({
    name,
    config,
    ...
  }: let
    instance = config;
  in {
    options = {
      enable = lib.mkEnableOption "Gatus instance ${name}";
      package = lib.mkPackageOption pkgs "gatus" {};
      port = mkOption {
        type = types.port;
        description = "Loopback listener port.";
      };
      stateDirectory = mkOption {
        type = types.str;
        default = "gatus-${name}";
        description = "Unique directory name under /var/lib for SQLite history.";
      };
      title = mkOption {
        type = types.str;
        description = "Dashboard title.";
      };
      header = mkOption {
        type = types.str;
        default = instance.title;
        defaultText = lib.literalExpression "config.title";
        description = "Dashboard header.";
      };
      buttons = mkOption {
        type = types.listOf button;
        default = [];
        description = "Dashboard header buttons.";
      };
      selection = mkOption {
        type = types.nullOr selection;
        default = null;
        description = "Select generic service profiles through Fleetix.";
      };
      endpoints = mkOption {
        type = types.listOf (types.attrsOf types.anything);
        default = [];
        description = "Gatus network checks; populated automatically with selection.";
      };
      environmentFile = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Runtime environment file; GATUS_EXTERNAL_TOKEN authenticates host-local health publications.";
      };
      client = mkOption {
        type = types.attrsOf types.anything;
        default = {};
        description = "Defaults merged into each network probe client, such as a private DNS resolver.";
      };
      trustedCertificateFiles = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Runtime CA files loaded by systemd credentials in addition to the system certificate bundle.";
      };
      externalChecks = mkOption {
        type = types.listOf (types.attrsOf types.anything);
        readOnly = true;
        default =
          if instance.enable && instance.selection != null
          then inventories.${name}.externalChecks
          else [];
        defaultText = lib.literalExpression "the selected Fleetix host-local checks";
        description = "Selected host-local checks, including stable Gatus keys and source contracts.";
      };
      excluded = mkOption {
        type = types.listOf (types.attrsOf types.str);
        readOnly = true;
        default =
          if instance.enable && instance.selection != null
          then inventories.${name}.excluded
          else [];
        defaultText = lib.literalExpression "the Fleetix lifecycle exclusions";
        description = "Services explicitly excluded by lifecycle or health policy.";
      };
      settings = mkOption {
        type = types.attrsOf types.anything;
        readOnly = true;
        description = "Rendered Gatus configuration.";
      };
    };
    config = lib.mkMerge [
      {
        settings =
          {
            web = {
              address = "127.0.0.1";
              inherit (instance) port;
            };
            storage = {
              type = "sqlite";
              path = "/var/lib/${instance.stateDirectory}/gatus.db";
            };
            ui =
              {
                inherit (instance) title header;
                default-sort-by = "group";
              }
              // lib.optionalAttrs (instance.buttons != []) {inherit (instance) buttons;};
            endpoints = map (endpoint: endpoint // {client = instance.client // (endpoint.client or {});}) instance.endpoints;
          }
          // lib.optionalAttrs (instance.externalChecks != []) {
            external-endpoints = map (check: check.settings // {token = "\${GATUS_EXTERNAL_TOKEN}";}) instance.externalChecks;
          };
      }
      (lib.mkIf (instance.enable && instance.selection != null) {
        inherit (inventories.${name}) endpoints;
      })
    ];
  });
  instances = builtins.attrValues enabled;
in {
  options.services = {
    gatusInstances = mkOption {
      type = types.attrsOf instanceType;
      default = {};
      description = "Loopback-only Gatus instances with isolated state.";
    };
    gatusDiscovery = {
      topology = mkOption {
        type = types.attrs;
        default = {};
        description = "Fleetix topology through the consumer's topology facade.";
      };
      domains = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Domain affiliations, matched on DNS label boundaries.";
      };
      defaultDomain = mkOption {
        type = types.str;
        default = "";
        description = "Affiliation of shared services without a site.";
      };
      requireCompleteProfiles = mkOption {
        type = types.bool;
        default = true;
        description = "Require a profile for every endpoint and HTTP site.";
      };
    };
  };
  config = lib.mkIf (enabled != {}) {
    assertions =
      [
        {
          assertion = lib.all (i: i.endpoints != []) instances;
          message = "Gatus v5 requires at least one network endpoint per enabled instance, including instances with external checks.";
        }
        {
          assertion = builtins.length (lib.unique (map (i: i.port) instances)) == builtins.length instances;
          message = "gatusInstances requires unique listener ports.";
        }
        {
          assertion = builtins.length (lib.unique (map (i: i.stateDirectory) instances)) == builtins.length instances;
          message = "gatusInstances requires separate state directories.";
        }
        {
          assertion = lib.all (i: builtins.match "^[A-Za-z0-9_-]+$" i.stateDirectory != null) instances;
          message = "gatusInstances stateDirectory must be a bare directory name.";
        }
        {
          assertion = lib.all (i: i.externalChecks == [] || i.environmentFile != null) instances;
          message = "Gatus host-local checks require a runtime GATUS_EXTERNAL_TOKEN environment file.";
        }
        {
          assertion = builtins.length (lib.filter (i: i.selection.includeInternal) (builtins.attrValues discovered)) <= 1;
          message = "Only one Gatus instance may collect restricted/shared service checks.";
        }
      ]
      ++ lib.optionals (discovered != {} && discovery.requireCompleteProfiles) [
        {
          assertion = coverage.unprofiledEndpoints == [] && coverage.unprofiledSites == [];
          message = "Gatus discovery has unprofiled endpoints ${toString coverage.unprofiledEndpoints} or sites ${toString coverage.unprofiledSites}.";
        }
        {
          assertion = lib.sort builtins.lessThan expectedChecks == lib.sort builtins.lessThan selectedChecks;
          message = "Gatus discovery requires every active check to be selected exactly once; inspect instance domain selectors and includeInternal.";
        }
      ];
    systemd.services = lib.mapAttrs' (name: instance:
      lib.nameValuePair "gatus-${name}" {
        description = "Gatus dashboard ${name}";
        after = ["network-online.target"];
        requires = ["network-online.target"];
        wantedBy = ["multi-user.target"];
        serviceConfig =
          {
            DynamicUser = true;
            User = "gatus-${name}";
            Group = "gatus-${name}";
            Type = "simple";
            Restart = "on-failure";
            ExecStart = lib.getExe instance.package;
            StateDirectory = instance.stateDirectory;
            SyslogIdentifier = "gatus-${name}";
            NoNewPrivileges = true;
            LoadCredential = lib.imap0 (index: path: "ca-${toString index}.crt:${path}") instance.trustedCertificateFiles;
          }
          // lib.optionalAttrs (instance.environmentFile != null) {EnvironmentFile = instance.environmentFile;};
        environment =
          {
            GATUS_CONFIG_PATH = (pkgs.formats.yaml {}).generate "gatus-${name}.yaml" instance.settings;
            SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
          }
          // lib.optionalAttrs (instance.trustedCertificateFiles != []) {SSL_CERT_DIR = "%d";};
      })
    enabled;
  };
}
