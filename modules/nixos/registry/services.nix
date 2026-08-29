{
  config,
  inputs ? {},
  lib,
  ...
}: let
  inherit (lib) mkIf mkMerge mkOption types;
  fleetixLib = inputs.fleetix.lib or (throw "canix-toolbelt service-registry: inputs.fleetix.lib is required when canix-toolbelt.fleetix.enable = true");

  pathMatchSubmodule = types.submodule {
    options = {
      type = mkOption {
        type = types.enum ["exact" "prefix"];
        description = "Path match kind.";
      };
      value = mkOption {
        type = types.str;
        description = "HTTP path value.";
      };
    };
  };

  httpMatchSubmodule = types.submodule {
    options = {
      paths = mkOption {
        type = types.listOf pathMatchSubmodule;
        default = [];
        description = "Ordered exact or prefix path matches.";
      };
      absentQueryParams = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Query parameters that must be absent.";
      };
    };
  };

  actionSubmodule = types.submodule {
    options = {
      type = mkOption {
        type = types.enum ["proxy" "files" "redirect" "respond"];
        description = "HTTP action tag.";
      };
      endpoint = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Endpoint registry key.";
      };
      stripPrefix = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional path prefix stripped before proxying.";
      };
      rootRef = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Host-local Caddy static root key.";
      };
      indexNames = mkOption {
        type = types.listOf types.str;
        default = ["index.html"];
        description = "Ordered index file names.";
      };
      to = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Redirect target.";
      };
      status = mkOption {
        type = types.nullOr (types.ints.between 100 599);
        default = null;
        description = "Redirect or response status.";
      };
      preserveUri = mkOption {
        type = types.bool;
        default = true;
        description = "Append the original request URI to the target.";
      };
      body = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional response body.";
      };
    };
  };

  routeSubmodule = types.submodule {
    options = {
      match = mkOption {
        type = httpMatchSubmodule;
        description = "HTTP request match.";
      };
      action = mkOption {
        type = actionSubmodule;
        description = "Tagged HTTP route action.";
      };
      authPolicy = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Opaque key in canix-toolbelt.services.caddy.authProviders.";
      };
      responseHeaders = mkOption {
        type = types.attrsOf (types.listOf types.str);
        default = {};
        description = "Response headers set by this route.";
      };
    };
  };

  endpointSubmodule = types.submodule {
    options = {
      targetHost = mkOption {
        type = types.str;
        description = "Host running the endpoint.";
      };
      port = mkOption {
        type = types.port;
        description = "Endpoint TCP port.";
      };
      transport = mkOption {
        type = types.enum ["tcp" "http" "https" "h2c"];
        description = "Endpoint transport.";
      };
      bind = mkOption {
        type = types.enum ["loopback" "lan"];
        description = "Endpoint bind scope.";
      };
      remoteVia = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "LAN endpoint used to relay remote access to this loopback endpoint.";
      };
      tlsServerName = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional upstream TLS SNI.";
      };
      tcpProbe = mkOption {
        type = types.bool;
        default = true;
        description = "Whether TCP health probing is enabled.";
      };
    };
  };

  siteSubmodule = types.submodule {
    options = {
      hostname = mkOption {
        type = types.str;
        description = "HTTP site hostname.";
      };
      ingress = mkOption {
        type = types.str;
        description = "Ingress group key.";
      };
      access = mkOption {
        type = types.enum ["cloudflare" "direct" "vpn"];
        description = "Site access policy.";
      };
      dnsPublication = mkOption {
        type = types.enum ["managed" "external" "none"];
        description = "DNS publication policy.";
      };
      routes = mkOption {
        type = types.listOf routeSubmodule;
        description = "Ordered HTTP routes.";
      };
    };
  };

  ingressGroupSubmodule = types.submodule {
    options = {
      scope = mkOption {
        type = types.enum ["public" "vpn"];
        description = "Ingress network scope.";
      };
      hosts = mkOption {
        type = types.listOf types.str;
        description = "Hosts serving this ingress group.";
      };
      sourceIps = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Additional source IPs used by ingress hosts, such as a failover VIP.";
      };
    };
  };

  cfg = config.canix-toolbelt.services;
  hostname = config.networking.hostName;
  host = config.canix-toolbelt.hosts.${hostname} or {};
  lanInterface = host.lanInterface or null;

  actionAssertions = lib.concatMap (siteName:
    map (route: let
      action = route.action;
      valid =
        if action.type == "proxy"
        then action.endpoint != null
        else if action.type == "files"
        then action.rootRef != null
        else if action.type == "redirect"
        then action.to != null && action.status != null && action.status >= 300 && action.status <= 399
        else action.status != null;
    in {
      assertion = valid;
      message = "canix-toolbelt service-registry: site `${siteName}` has an incomplete `${action.type}` action";
    })
    cfg.httpSites.${siteName}.routes) (builtins.attrNames cfg.httpSites);

  endpointAssertions =
    lib.mapAttrsToList (name: endpoint: let
      relay =
        if endpoint.remoteVia == null
        then null
        else cfg.endpoints.${endpoint.remoteVia} or null;
    in {
      assertion =
        endpoint.remoteVia
        == null
        || (endpoint.bind == "loopback" && relay != null && relay.bind == "lan" && relay.targetHost == endpoint.targetHost);
      message = "canix-toolbelt service-registry: endpoint `${name}` remoteVia must reference a LAN endpoint on the same target host";
    })
    cfg.endpoints;

  proxyUses = lib.concatMap (siteName: let
    site = cfg.httpSites.${siteName};
  in
    map (route: {
      inherit siteName site route;
      endpointName = route.action.endpoint;
    }) (builtins.filter (route: route.action.type == "proxy") site.routes)) (builtins.attrNames cfg.httpSites);

  directEndpointNames = lib.unique (map (use: use.endpointName) proxyUses);
  relayNames = lib.unique (builtins.filter (name: name != null) (map (use: let
    endpoint = cfg.endpoints.${use.endpointName} or null;
  in
    if endpoint == null
    then null
    else endpoint.remoteVia)
  proxyUses));
  relayOnlyNames = builtins.filter (name: !(builtins.elem name directEndpointNames)) relayNames;

  lanPorts = lib.unique (map (name: cfg.endpoints.${name}.port) (builtins.filter (name: let
    endpoint = cfg.endpoints.${name};
  in
    endpoint.targetHost
    == hostname
    && endpoint.bind == "lan"
    && !(builtins.elem name relayOnlyNames)) (builtins.attrNames cfg.endpoints)));

  relaySourceIps = relayName:
    lib.unique (builtins.filter (ip: ip != null) (lib.concatMap (use: let
      endpoint = cfg.endpoints.${use.endpointName} or null;
      group = cfg.ingressGroups.${use.site.ingress} or null;
    in
      if endpoint == null || endpoint.remoteVia != relayName || group == null
      then []
      else
        map (ingressHost: (config.canix-toolbelt.hosts.${ingressHost} or {}).lanIp or null) (builtins.filter (ingressHost: ingressHost != hostname) group.hosts)
        ++ group.sourceIps)
    proxyUses));

  relayRules = lib.concatMapStringsSep "\n" (relayName: let
    endpoint = cfg.endpoints.${relayName};
    sourceIps = relaySourceIps relayName;
  in
    lib.optionalString (endpoint.targetHost == hostname && sourceIps != [] && lanInterface != null) ''
      iifname "${lanInterface}" ip saddr { ${lib.concatStringsSep ", " sourceIps} } tcp dport ${toString endpoint.port} accept
    '')
  relayOnlyNames;
in {
  options.canix-toolbelt.services = {
    endpoints = mkOption {
      type = types.attrsOf endpointSubmodule;
      default = {};
      description = "Fleetix v2 service endpoints.";
    };

    httpSites = mkOption {
      type = types.attrsOf siteSubmodule;
      default = {};
      description = "Fleetix v2 HTTP sites.";
    };

    ingressGroups = mkOption {
      type = types.attrsOf ingressGroupSubmodule;
      default = {};
      description = "Fleetix v2 deployment ingress groups.";
    };

    endpointsForHost = mkOption {
      type = types.attrsOf endpointSubmodule;
      readOnly = true;
      default = lib.filterAttrs (_: endpoint: endpoint.targetHost == hostname) cfg.endpoints;
      description = "Endpoints running on this host.";
    };

    sitesForCurrentHost = mkOption {
      type = types.attrsOf siteSubmodule;
      readOnly = true;
      default = lib.filterAttrs (_: site: builtins.elem hostname (cfg.ingressGroups.${site.ingress}.hosts or [])) cfg.httpSites;
      description = "HTTP sites assigned to an ingress group served by this host.";
    };
  };

  config = mkMerge [
    (mkIf config.canix-toolbelt.fleetix.enable (let
      ft =
        if config.canix-toolbelt.fleetix.topology != null
        then config.canix-toolbelt.fleetix.topology
        else config.fleetix.topology;
      normalized = fleetixLib.projections.normalize {topology = ft;};
    in {
      assertions = [
        {
          assertion = (normalized.topology.schemaVersion or null) == 2;
          message = "canix-toolbelt service-registry requires Fleetix topology.schemaVersion = 2";
        }
      ];

      canix-toolbelt.services = {
        inherit (normalized.topology.services) endpoints httpSites;
        ingressGroups = normalized.topology.deployment.ingressGroups or {};
      };
    }))

    {
      assertions = actionAssertions ++ endpointAssertions;
      networking.firewall = {
        interfaces = lib.optionalAttrs (lanInterface != null && lanPorts != []) {
          ${lanInterface}.allowedTCPPorts = lanPorts;
        };
        extraInputRules = lib.mkAfter relayRules;
      };
    }
  ];
}
