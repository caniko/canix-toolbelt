{
  config,
  lib,
  pkgs,
  ...
}: let
  dns = config.canix-toolbelt.dns;
  cfg = dns.ddns;
  dnsLib = import ../../../lib/dns.nix;
  providers = import ../../../lib/dns-providers.nix;
  networkTypes = import ../../../lib/network-types.nix {inherit lib;};
  inherit (lib) mkIf mkOption types;

  hosts = lib.mapAttrs (_: host: let
    provider = dns.providers.${host.provider} or null;
    meta =
      if provider == null
      then null
      else providers.${provider.kind};
    fqdn = dnsLib.normalizeHostname lib host.fqdn;
    zone = dnsLib.zoneForHost {
      inherit lib fqdn;
      zones = builtins.attrNames dns.zones;
    };
    path =
      if provider == null
      then null
      else if provider.ddnsTokenPath != null
      then provider.ddnsTokenPath
      else provider.token.secretPath;
  in
    host // {inherit meta fqdn zone path;})
  cfg.hosts;
  ready = lib.filterAttrs (_: host:
    host.meta
    != null
    && host.path != null
    && host.families != []
    && host.zone != null
    && (!host.proxied || host.meta.supportsProxy)
    && builtins.elem host.provider [dns.zones.${host.zone}.provider dns.zones.${host.zone}.migrateTo])
  hosts;
  cloudflare = lib.filterAttrs (_: host: host.meta.ddnsKind == "cloudflare") ready;
  others = lib.filterAttrs (_: host: host.meta.ddnsKind != "cloudflare") ready;
  cloudflarePaths = lib.unique (map (host: host.path) (lib.attrValues cloudflare));
  forFamily = family: map (host: host.fqdn) (lib.filter (host: builtins.elem family host.families) (lib.attrValues cloudflare));
  ip4 = forFamily "ipv4";
  ip6 = forFamily "ipv6";
  ownedDomains = family:
    map (host: host.fqdn)
    (lib.filter (host: host.meta.ddnsKind == "desec" || builtins.elem family host.families) (lib.attrValues ready));
  excludeDrift = dnsLib.ddnsExcludeDrift {
    inherit lib;
    inherit (dns) zones;
    ip4Domains = ownedDomains "ipv4";
    ip6Domains = ownedDomains "ipv6";
  };
in {
  options.canix-toolbelt.dns.ddns = {
    enable = lib.mkEnableOption "periodic updates of dynamic DNS records";
    hosts = mkOption {
      type = types.attrsOf (types.submodule {
        options = {
          fqdn = mkOption {
            type = networkTypes.hostname;
            description = "Fully qualified DNS name to update.";
          };
          provider = mkOption {
            type = types.str;
            default = "cloudflare";
            description = "Provider instance ID, matching one of the zone's targets.";
          };
          families = mkOption {
            type = types.listOf (types.enum ["ipv4" "ipv6"]);
            default = ["ipv4" "ipv6"];
            description = "Address families to publish. deSEC clears records for omitted families.";
          };
          proxied = mkOption {
            type = types.bool;
            default = false;
            description = "Proxy the hostname through Cloudflare.";
          };
        };
      });
      default = {};
      description = "DDNS hostnames keyed by service ID. Exclude their managed records from octoDNS reconciliation.";
    };
  };

  config = mkIf cfg.enable {
    assertions =
      lib.concatLists (lib.mapAttrsToList (id: host: [
          {
            assertion = builtins.match "[A-Za-z0-9_-]+" id != null;
            message = "canix-toolbelt.dns.ddns.hosts: '${id}' is not a valid service ID";
          }
          {
            assertion = host.zone != null && builtins.elem host.provider [dns.zones.${host.zone}.provider dns.zones.${host.zone}.migrateTo];
            message = "canix-toolbelt.dns.ddns: '${host.fqdn}' must use a provider targeted by its declared zone";
          }
          {
            assertion = host.meta != null && host.path != null;
            message = "canix-toolbelt.dns.ddns: '${host.fqdn}' requires providers.${host.provider}.ddnsTokenPath or token.secretPath";
          }
          {
            assertion = host.families != [];
            message = "canix-toolbelt.dns.ddns: '${host.fqdn}' requires at least one address family";
          }
          {
            assertion = !host.proxied || (host.meta != null && host.meta.supportsProxy);
            message = "canix-toolbelt.dns.ddns: '${host.fqdn}' uses a provider that does not support proxying";
          }
        ])
        hosts)
      ++ lib.optionals dns.enable (map (record: {
        assertion = false;
        message = "canix-toolbelt.dns.ddns: ${record.fqdn} needs a ${record.type} octoDNS exclude in '${record.zone}'";
      }) (lib.unique excludeDrift.missingExcludes))
      ++ [
        {
          assertion = builtins.length cloudflarePaths <= 1;
          message = "canix-toolbelt.dns.ddns: services.cloudflare-ddns supports only one Cloudflare credential file per host";
        }
        {
          assertion = builtins.length (lib.attrValues hosts) == builtins.length (lib.unique (map (host: [host.provider host.fqdn]) (lib.attrValues hosts)));
          message = "canix-toolbelt.dns.ddns: configure each hostname at most once per provider";
        }
      ];

    services.cloudflare-ddns = mkIf (builtins.length cloudflarePaths == 1) {
      enable = true;
      ip4Domains = ip4;
      ip6Domains = ip6;
      provider.ipv4 = lib.mkIf (ip4 == []) "none";
      provider.ipv6 = lib.mkIf (ip6 == []) "none";
      proxied = dnsLib.cloudflareDdnsProxiedExpression {dynamicHosts = lib.attrValues cloudflare;};
      # The upstream module requires EnvironmentFile; this one contains no secrets.
      credentialsFile = pkgs.emptyFile;
    };

    systemd = {
      services =
        lib.mapAttrs' (id: host:
          lib.nameValuePair "dns-ddns-${id}" {
            description = "Update dynamic DNS for ${host.fqdn} (${host.meta.ddnsKind})";
            after = ["network-online.target"];
            wants = ["network-online.target"];
            environment.SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
            serviceConfig = {
              Type = "oneshot";
              DynamicUser = true;
              LoadCredential = ["${host.meta.octodnsDefaultEnv}:${host.path}"];
              ExecStart =
                lib.escapeShellArgs [
                  "${pkgs.python3}/bin/python"
                  "${./dns-ddns.py}"
                  host.meta.ddnsKind
                  host.meta.octodnsDefaultEnv
                  (dnsLib.normalizeHostname lib host.zone)
                  host.fqdn
                ]
                + " "
                + lib.escapeShellArgs (lib.unique host.families);
              TimeoutStartSec = "2min";
              NoNewPrivileges = true;
              ProtectSystem = "strict";
              ProtectHome = true;
            };
          })
        others
        // lib.optionalAttrs (builtins.length cloudflarePaths == 1) {
          cloudflare-ddns.serviceConfig = let
            name = providers.cloudflare.octodnsDefaultEnv;
          in {
            LoadCredential = ["${name}:${builtins.head cloudflarePaths}"];
            Environment = ["${name}_FILE=%d/${name}"];
          };
        };
      timers = lib.mapAttrs' (id: _:
        lib.nameValuePair "dns-ddns-${id}" {
          wantedBy = ["timers.target"];
          timerConfig = {
            OnBootSec = "30s";
            OnUnitInactiveSec = "5min";
          };
        })
      others;
    };
  };
}
