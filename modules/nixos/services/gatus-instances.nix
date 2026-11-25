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
    attachAlerts = item:
      if item ? alerts
      then item
      else item // lib.optionalAttrs (instance.defaultAlerts != []) {alerts = instance.defaultAlerts;};
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
      storage = {
        type = mkOption {
          type = types.enum ["sqlite" "postgres" "memory"];
          default = "sqlite";
          description = "Gatus storage backend; memory keeps no history and writes nothing to disk.";
        };
        path = mkOption {
          type = types.nullOr types.str;
          default = null;
          description = "SQLite file path (null means /var/lib/<stateDirectory>/gatus.db), or the required PostgreSQL connection string for postgres; ignored for memory. The rendered config is world-readable in the Nix store, so keep credentials out of it: write `\${VAR}` and supply VAR through environmentFile or environmentFiles (Gatus expands environment variables).";
        };
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
      environmentFiles = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Additional runtime systemd EnvironmentFile paths, loaded in order after environmentFile if set.";
      };
      alerting = mkOption {
        type = types.attrsOf (types.attrsOf types.anything);
        default = {};
        description = ''
          Top-level Gatus alerting provider attribute sets. Use supported provider keys
          (e.g. ntfy, email, custom, googlechat). Gatus silently ignores unrecognized
          provider keys and validates required fields of recognized providers at startup.
          For both defaultAlerts and per-endpoint alerts, the module checks that every alert
          has a non-empty string type and that enabled alerts reference a non-empty provider
          attribute set here.
          Rendered YAML is world-readable in the Nix store. Use `''${VAR}` placeholders for
          credentials and supply VAR at runtime via environmentFile or environmentFiles.
          Gatus substitutes these values before parsing YAML, so secrets must be YAML-safe
          (for example, opaque tokens without bare colons, quotes, or newlines).
        '';
      };
      defaultAlerts = mkOption {
        type = types.listOf (types.attrsOf types.anything);
        default = [];
        description = "Gatus alert entries for network and external endpoints without an alerts attribute. An explicit alerts list replaces these defaults; null or an empty list opts out.";
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
            storage =
              {inherit (instance.storage) type;}
              // lib.optionalAttrs (instance.storage.type != "memory") {
                path =
                  if instance.storage.path != null
                  then instance.storage.path
                  else "/var/lib/${instance.stateDirectory}/gatus.db";
              };
            ui =
              {
                inherit (instance) title header;
                default-sort-by = "group";
              }
              // lib.optionalAttrs (instance.buttons != []) {inherit (instance) buttons;};
            endpoints = map (endpoint: attachAlerts (endpoint // {client = instance.client // (endpoint.client or {});})) instance.endpoints;
          }
          // lib.optionalAttrs (instance.alerting != {}) {inherit (instance) alerting;}
          // lib.optionalAttrs (instance.externalChecks != []) {
            external-endpoints = map (check: attachAlerts (check.settings // {token = "\${GATUS_EXTERNAL_TOKEN}";})) instance.externalChecks;
          };
      }
      (lib.mkIf (instance.enable && instance.selection != null) {
        inherit (inventories.${name}) endpoints;
      })
    ];
  });
  instances = builtins.attrValues enabled;
  validAlertType = alert: builtins.isAttrs alert && builtins.isString (alert.type or null) && alert.type != "";
  validEndpointAlerts = item: !(item ? alerts) || item.alerts == null || builtins.isList item.alerts;
  endpointAlerts = item:
    if (item ? alerts) && builtins.isList item.alerts
    then item.alerts
    else [];
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
          assertion = lib.all (i: i.storage.type != "postgres" || i.storage.path != null) instances;
          message = "gatusInstances storage.type = \"postgres\" requires storage.path (the connection string).";
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
          assertion = lib.all (i: i.externalChecks == [] || i.environmentFile != null || i.environmentFiles != []) instances;
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
      ]
      ++ lib.concatLists (lib.mapAttrsToList (name: i: let
          checkedEndpoints = i.settings.endpoints ++ (i.settings.external-endpoints or []);
          alerts = i.defaultAlerts ++ lib.concatMap endpointAlerts checkedEndpoints;
        in [
          {
            assertion = lib.all validEndpointAlerts checkedEndpoints;
            message = "gatusInstances.${name}: endpoint alerts must be a list or null.";
          }
          {
            assertion = lib.all validAlertType alerts;
            message = "gatusInstances.${name}: every alert must have a non-empty string type.";
          }
          {
            assertion = lib.all (alert:
              !validAlertType alert
              || (alert.enabled or true) == false
              || (let
                provider = i.alerting.${alert.type} or null;
              in
                builtins.isAttrs provider && provider != {}))
            alerts;
            message = "gatusInstances.${name}: every enabled alert must reference a non-empty provider attrset in alerting.";
          }
        ])
        enabled);
    systemd.services = lib.mapAttrs' (name: instance: let
      envFiles = lib.optional (instance.environmentFile != null) instance.environmentFile ++ instance.environmentFiles;
    in
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
          // lib.optionalAttrs (envFiles != []) {
            EnvironmentFile =
              if instance.environmentFiles == []
              then instance.environmentFile
              else envFiles;
          };
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
