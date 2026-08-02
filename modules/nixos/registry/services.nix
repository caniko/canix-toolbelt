{
  config,
  inputs ? {},
  lib,
  ...
}: let
  inherit (lib) filter mkIf mkMerge mkOption types;
  fleetixLib = inputs.fleetix.lib or (throw "canix-toolbelt service-registry: inputs.fleetix.lib is required when canix-toolbelt.fleetix.enable = true");

  serviceRoutes = service:
    if service.routes == []
    then [
      {
        targetHost = service.targetHost;
        port = service.port;
      }
    ]
    else service.routes;

  routeTargetHost = service: route:
    if route.targetHost != null
    then route.targetHost
    else service.targetHost;

  routePort = service: route:
    if route.port != null
    then route.port
    else service.port;

  authSubmodule = types.submodule {
    options = {
      enable = lib.mkEnableOption "OIDC authentication via caddy-security";

      provider = mkOption {
        type = types.enum ["kanidm" "rauthy"];
        default = "kanidm";
        description = "OIDC identity provider backend.";
      };
    };
  };

  metricEndpointSubmodule = types.submodule {
    options = {
      port = mkOption {
        type = types.port;
        description = "Metrics endpoint port.";
      };
      path = mkOption {
        type = types.str;
        default = "/metrics";
        description = "Metrics endpoint HTTP path.";
      };
      scheme = mkOption {
        type = types.enum ["http" "https"];
        default = "http";
        description = "Metrics endpoint scheme.";
      };
    };
  };

  monitoringSubmodule = types.submodule {
    options = {
      probe = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional blackbox probe kind override.";
      };
      metrics = mkOption {
        type = types.listOf metricEndpointSubmodule;
        default = [];
        description = "Metrics endpoints associated with this service.";
      };
      alert = mkOption {
        type = types.nullOr types.bool;
        default = null;
        description = "Optional alerting intent for this service.";
      };
    };
  };

  reverseProxyRouteSubmodule = types.submodule {
    options = {
      paths = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Ordered Caddy path matchers; an empty list is the fallback route.";
      };
      targetHost = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional route-specific backend host.";
      };
      port = mkOption {
        type = types.nullOr types.port;
        default = null;
        description = "Optional route-specific backend port.";
      };
      upstreamScheme = mkOption {
        type = types.nullOr (types.enum ["http" "https"]);
        default = null;
        description = "Optional route-specific backend scheme.";
      };
      tlsServerName = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional route-specific upstream TLS SNI.";
      };
      stripPrefix = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional path prefix to strip before proxying.";
      };
      monitoringIdentity = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Stable monitoring identity for this backend route.";
      };
    };
  };

  reverseProxyServiceSubmodule = types.submodule {
    options = {
      name = mkOption {
        type = types.str;
        description = "Service identifier";
      };

      auth = mkOption {
        type = authSubmodule;
        default = {enable = false;};
        description = "OIDC authentication configuration for this service route.";
      };
      hostname = mkOption {
        type = types.str;
        description = "Public hostname for reverse proxy";
      };
      port = mkOption {
        type = types.port;
        description = "Port the service listens on";
      };
      upstreamScheme = mkOption {
        type = types.enum ["http" "https"];
        default = "http";
        description = ''
          Transport scheme Caddy uses to dial the upstream. "https" makes Caddy
          re-encrypt to a TLS-terminating backend (e.g. kanidm on 127.0.0.1:8443),
          emitting reverse_proxy transport.protocol="http" with a tls block.
        '';
      };
      tlsServerName = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = ''
          SNI / tls.server_name sent to an upstreamScheme="https" backend.
          Defaults to the public hostname when null.
        '';
      };
      local = mkOption {
        type = types.nullOr types.bool;
        default = null;
        description = ''
          Override loopback dialing. When null (default) it is computed: a
          service whose targetHost equals this Caddy host's networking.hostName
          dials 127.0.0.1 instead of the host lanIp, matching hand-written
          loopback routes (and avoiding localhost→IPv6 mismatches). Set
          true/false to force.
        '';
      };
      targetHost = mkOption {
        type = types.str;
        description = "Hostname where the service runs (key from canix-toolbelt.hosts)";
      };
      lanExposed = mkOption {
        type = types.bool;
        default = false;
        description = "Whether to open the service port on the target host's LAN interface.";
      };
      serviceHost = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional service-host label for DNS or routing metadata.";
      };
      zone = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional DNS zone associated with this service.";
      };
      proxied = mkOption {
        type = types.bool;
        default = false;
        description = "Whether the service binds to localhost and needs a local reverse proxy";
      };
      cloudflareProxied = mkOption {
        type = types.bool;
        default = true;
        description = "Whether traffic goes through Cloudflare proxy. If false, the hostname is exempt from the Cloudflare CIDR allowlist.";
      };
      publishCname = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Whether the canix.dns synthesizer should emit a public CNAME
          for this service's hostname. Set false for services that are
          reverse-proxied locally but not externally addressable by name
          (admin-only, behind another auth layer, etc.).
        '';
      };
      vpnOnly = mkOption {
        type = types.bool;
        default = false;
        description = "Whether this service is only reachable via VPN DNS (vpn.candee.baby zone).";
      };
      dnsComment = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional provider-side comment for synthesized public DNS records.";
      };
      monitoring = mkOption {
        type = monitoringSubmodule;
        default = {};
        description = "Monitoring metadata for this service.";
      };
      routes = mkOption {
        type = types.listOf reverseProxyRouteSubmodule;
        default = [];
        description = "Ordered route backends; an empty list preserves the legacy service route.";
      };
    };
  };

  internalServiceSubmodule = types.submodule {
    options = {
      name = mkOption {
        type = types.str;
        description = "Internal service identifier";
      };
      port = mkOption {
        type = types.port;
        description = "Port the internal service listens on";
      };
      targetHost = mkOption {
        type = types.str;
        description = "Hostname where the service runs";
      };
      description = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Human-readable service description.";
      };
      monitoring = mkOption {
        type = monitoringSubmodule;
        default = {};
        description = "Monitoring metadata for this service.";
      };
    };
  };

  staticFileServiceSubmodule = types.submodule {
    options = {
      name = mkOption {
        type = types.str;
        description = "Static file service identifier";
      };
      hostname = mkOption {
        type = types.str;
        description = "Public hostname for the static file route";
      };
      staticRoot = mkOption {
        type = types.nullOr types.path;
        default = null;
        description = "Directory served as the static file root";
      };
      cloudflareProxied = mkOption {
        type = types.bool;
        default = true;
        description = "Whether traffic goes through Cloudflare proxy. If false, the hostname is exempt from the Cloudflare CIDR allowlist.";
      };
      vpnOnly = mkOption {
        type = types.bool;
        default = false;
        description = "Whether this service is only reachable via VPN DNS (vpn.candee.baby zone).";
      };
      dnsComment = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional provider-side comment for synthesized public DNS records.";
      };
      zone = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional DNS zone associated with this service.";
      };
      kind = mkOption {
        type = types.nullOr types.str;
        default = "static";
        description = "Kind of static file service (e.g. 'static-file' for Caddy file_server routes).";
      };
      monitoring = mkOption {
        type = monitoringSubmodule;
        default = {};
        description = "Monitoring metadata for this service.";
      };
    };
  };
in {
  options.canix-toolbelt.services = {
    sshPort = mkOption {
      type = types.port;
      default = 1337;
      description = "Canonical SSH port for hosts in this registry.";
    };

    hostSshKeyPath = mkOption {
      type = types.str;
      default = "/etc/ssh/id_ed25519";
      description = "Canonical host SSH private key path.";
    };

    hostSshPubKeyPath = mkOption {
      type = types.str;
      default = "/etc/ssh/id_ed25519.pub";
      description = "Canonical host SSH public key path.";
    };

    reverseProxyServices = mkOption {
      type = types.listOf reverseProxyServiceSubmodule;
      default = [];
      description = "Services that need reverse proxy routing and firewall rules";
    };

    staticFileServices = mkOption {
      type = types.listOf staticFileServiceSubmodule;
      default = [];
      description = "Static file services rendered as Caddy file_server routes";
    };

    internalServices = mkOption {
      type = types.listOf internalServiceSubmodule;
      default = [];
      description = "Internal service registry entries that are not public reverse-proxy routes.";
    };

    emailIdentities = mkOption {
      type = types.attrsOf types.str;
      default = {};
      description = "Fleet-level service email identities.";
    };

    localServices = mkOption {
      type = types.listOf reverseProxyServiceSubmodule;
      readOnly = true;
      default =
        filter (svc: svc.targetHost == config.networking.hostName)
        config.canix-toolbelt.services.reverseProxyServices;
      description = "Services running on this host (computed from reverseProxyServices)";
    };

    proxiedLocalServices = mkOption {
      type = types.listOf reverseProxyServiceSubmodule;
      readOnly = true;
      default =
        filter (svc: svc.proxied)
        config.canix-toolbelt.services.localServices;
      description = "Local services that need a local reverse proxy (proxied = true)";
    };
  };

  config = mkMerge [
    (mkIf config.canix-toolbelt.fleetix.enable (let
      ft =
        if config.canix-toolbelt.fleetix.topology != null
        then config.canix-toolbelt.fleetix.topology
        else config.fleetix.topology;
      normalized = fleetixLib.projections.normalize {topology = ft;};
      services = normalized.services;
    in {
      canix-toolbelt.services = {
        inherit (services) sshPort hostSshKeyPath hostSshPubKeyPath reverseProxyServices staticFileServices internalServices emailIdentities;
      };
    }))

    (let
      localPorts = lib.unique (lib.concatMap (
          service:
            map (route: routePort service route) (
              filter (route: routeTargetHost service route == config.networking.hostName) (serviceRoutes service)
            )
        )
        config.canix-toolbelt.services.reverseProxyServices);
    in {
      networking.firewall.allowedTCPPorts = localPorts;
    })

    (let
      hostname = config.networking.hostName;
      host = config.canix-toolbelt.hosts.${hostname} or {};
      lanInterface = host.lanInterface or null;
      lanExposedPorts = lib.unique (lib.concatMap (
        service:
          map (route: routePort service route) (
            filter (route: routeTargetHost service route == hostname) (serviceRoutes service)
          )
      ) (filter (service: service.lanExposed or false) config.canix-toolbelt.services.reverseProxyServices));
    in {
      networking.firewall.interfaces = lib.optionalAttrs (lanInterface != null) {
        ${lanInterface}.allowedTCPPorts = lanExposedPorts;
      };
    })
  ];
}
