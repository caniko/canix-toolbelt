{
  config,
  lib,
  ...
}: let
  caddyLib = import ../../../lib/caddy.nix {inherit lib;};
  cfg = config.canix-toolbelt.services.caddy;
  serviceCfg = config.canix-toolbelt.services;
  hostname = config.networking.hostName;

  resolveEndpoint = endpointName: let
    source = serviceCfg.endpoints.${endpointName} or (throw "canix-toolbelt Caddy registry: unknown endpoint `${endpointName}`");
    selectedName =
      if source.bind == "loopback" && source.targetHost != hostname
      then source.remoteVia or (throw "canix-toolbelt Caddy registry: loopback endpoint `${endpointName}` has no remoteVia endpoint for ingress host `${hostname}`")
      else endpointName;
    selected = serviceCfg.endpoints.${selectedName} or (throw "canix-toolbelt Caddy registry: endpoint `${endpointName}` references missing remoteVia endpoint `${selectedName}`");
    target = config.canix-toolbelt.hosts.${selected.targetHost} or {};
    address =
      if selected.bind == "loopback"
      then "127.0.0.1"
      else target.lanIp or (throw "canix-toolbelt Caddy registry: LAN endpoint `${selectedName}` target `${selected.targetHost}` has no lanIp");
  in
    selected
    // {
      name = selectedName;
      sourceEndpoint = endpointName;
      inherit address;
    };

  renderRoute = site: route:
    caddyLib.mkHttpRoute {
      inherit (site) hostname;
      inherit route;
      endpoint =
        if route.action.type == "proxy"
        then resolveEndpoint route.action.endpoint
        else null;
      staticRoots = cfg.staticRoots;
    };

  sitesForGroup = groupName:
    lib.filterAttrs (_: site: site.ingress == groupName) serviceCfg.httpSites;

  groupIsActive = groupName: group:
    builtins.elem hostname group.hosts
    && (cfg.ingressListeners.${groupName} or []) != [];

  ingressServers = lib.mapAttrs' (groupName: group: let
    sites = sitesForGroup groupName;
    siteValues = builtins.attrValues sites;
    cloudflareSites = builtins.filter (site: site.access == "cloudflare") siteValues;
    directHosts = map (site: site.hostname) (builtins.filter (site: site.access == "direct") siteValues);
  in
    lib.nameValuePair groupName {
      listen = cfg.ingressListeners.${groupName};
      routes = lib.concatMap (site: map (renderRoute site) site.routes) siteValues;
      cidrAllowlist = lib.optionals (group.scope == "public" && cloudflareSites != []) cfg.cloudflareCidrs;
      cidrExemptHosts = lib.optionals (group.scope == "public") directHosts;
    }) (lib.filterAttrs groupIsActive serviceCfg.ingressGroups);

  relayUses = lib.concatMap (siteName: let
    site = serviceCfg.httpSites.${siteName};
  in
    lib.concatMap (route:
      if route.action.type != "proxy"
      then []
      else let
        source = serviceCfg.endpoints.${route.action.endpoint} or null;
      in
        lib.optional (source != null && source.bind == "loopback" && source.remoteVia != null && source.targetHost == hostname) {
          inherit site source;
          sourceName = route.action.endpoint;
          relayName = source.remoteVia;
        })
    site.routes) (builtins.attrNames serviceCfg.httpSites);

  relayNames = lib.unique (map (use: use.relayName) relayUses);
  relayServers = builtins.listToAttrs (map (relayName: let
    relay = serviceCfg.endpoints.${relayName} or (throw "canix-toolbelt Caddy registry: missing relay endpoint `${relayName}`");
    relayHost = config.canix-toolbelt.hosts.${relay.targetHost} or {};
    address =
      if relay.targetHost != hostname || relay.bind != "lan"
      then throw "canix-toolbelt Caddy registry: relay endpoint `${relayName}` must be LAN-bound on `${hostname}`"
      else relayHost.lanIp or (throw "canix-toolbelt Caddy registry: relay endpoint `${relayName}` target `${relay.targetHost}` has no lanIp");
    uses = builtins.filter (use: use.relayName == relayName) relayUses;
  in
    lib.nameValuePair "relay-${relayName}" {
      listen = ["${address}:${toString relay.port}"];
      routes = lib.unique (map (use:
        caddyLib.mkRelayRoute {
          hostname = use.site.hostname;
          endpoint =
            use.source
            // {
              name = use.sourceName;
              address = "127.0.0.1";
            };
        })
      uses);
      metrics = false;
      automaticHttps = false;
    })
  relayNames);

  activeSites = builtins.attrValues (lib.filterAttrs (_: site: let
    group = serviceCfg.ingressGroups.${site.ingress} or null;
  in
    group != null && groupIsActive site.ingress group)
  serviceCfg.httpSites);
  activeRoutes = lib.concatMap (site: site.routes) activeSites;
  authPolicies = lib.unique (builtins.filter (policy: policy != null) (map (route: route.authPolicy) activeRoutes));
  caddySecurityPlugin = "github.com/greenpau/caddy-security@v1.1.62";

  siteAssertions =
    lib.mapAttrsToList (siteName: site: let
      group = serviceCfg.ingressGroups.${site.ingress} or null;
    in {
      assertion =
        group
        != null
        && ((site.access == "vpn") == (group.scope == "vpn"));
      message = "canix-toolbelt Caddy registry: site `${siteName}` must reference an ingress group whose scope matches access `${site.access}`";
    })
    serviceCfg.httpSites;

  routeAssertions = lib.concatMap (siteName: let
    site = serviceCfg.httpSites.${siteName};
  in
    map (route: let
      endpoint =
        if route.action.type == "proxy"
        then serviceCfg.endpoints.${route.action.endpoint} or null
        else null;
    in {
      assertion =
        route.action.type
        != "proxy"
        || (endpoint != null && endpoint.transport != "tcp");
      message = "canix-toolbelt Caddy registry: site `${siteName}` proxy routes require a non-TCP endpoint";
    })
    site.routes) (builtins.attrNames serviceCfg.httpSites);
in {
  imports = [./caddy-base.nix];

  config = lib.mkIf cfg.enable {
    assertions =
      siteAssertions
      ++ routeAssertions
      ++ map (policy: {
        assertion = builtins.hasAttr policy cfg.authProviders;
        message = "canix-toolbelt Caddy registry: authPolicy `${policy}` is missing from canix-toolbelt.services.caddy.authProviders";
      })
      authPolicies
      ++ lib.mapAttrsToList (groupName: group: {
        assertion =
          !groupIsActive groupName group
          || group.scope != "public"
          || (builtins.filter (site: site.access == "cloudflare") (builtins.attrValues (sitesForGroup groupName))) == []
          || cfg.cloudflareCidrs != [];
        message = "canix-toolbelt Caddy registry: public ingress group `${groupName}` serves Cloudflare sites but cloudflareCidrs is empty";
      })
      serviceCfg.ingressGroups;

    canix-toolbelt.services.caddy = {
      servers = ingressServers // relayServers;
      plugins = lib.mkIf (cfg.authProviders != {}) (lib.mkBefore [caddySecurityPlugin]);
    };
  };
}
