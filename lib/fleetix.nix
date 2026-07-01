{lib}: let
  stripPklClass = attrs:
    builtins.listToAttrs (
      builtins.filter (entry: entry.value != null) (
        builtins.map (name: {
          inherit name;
          value = attrs.${name};
        }) (builtins.filter (name: name != "__pkl_class") (builtins.attrNames attrs))
      )
    );

  firstExisting = attrs: names:
    lib.findFirst (name: builtins.hasAttr name attrs) null names;

  aliasValue = raw: target: let
    candidates =
      if builtins.isList target
      then target
      else [target];
    match = firstExisting raw candidates;
  in
    if match == null
    then
      if builtins.isString target && lib.hasInfix "." target
      then target
      else null
    else raw.${match};

  byNameFrom = services:
    builtins.listToAttrs (map (svc: {
        inherit (svc) name;
        value = svc;
      })
      services);

  mkWgEndpointHost = endpointSubdomain: zone: let
    endpoint =
      if endpointSubdomain == null
      then "wg"
      else endpointSubdomain;
  in
    if lib.hasInfix "." endpoint
    then endpoint
    else "${endpoint}-home.${zone}";

  firstLinkBinding = host: linkNames: let
    links = host.links or {};
    match = firstExisting links linkNames;
  in
    if match == null
    then {}
    else links.${match};
in rec {
  inherit stripPklClass;

  normalizeHosts = {
    topology,
    wgHomeLinkName ? "wg-home",
    directLinkNames ? ["direct-link"],
  }:
    builtins.mapAttrs (_name: host: let
      ln = host.links or {};
      wg = ln.${wgHomeLinkName} or {};
      direct = firstLinkBinding host directLinkNames;
    in
      host
      // {
        network =
          (host.network or {})
          // {
            wgHomeIp = wg.address or null;
            wgHomePublicKey =
              if (wg.role or "") == "server"
              then wg.publicKey or null
              else null;
            directLinkIp = direct.address or null;
            directLinkMac = direct.macAddress or null;
            directLinkInterface = direct.externalInterface or null;
          };
        wgHomeIp = wg.address or null;
        dataRoot = (host.storage or {}).dataRoot or null;
      })
    (topology.hosts or {});

  normalizeDomains = {
    topology,
    serviceHostAliases ? {},
  }: let
    tdom = topology.domains or {};
    tlinks = topology.links or {};
    tsvc = topology.services or {};
    zones = tdom.zones or ["example.invalid"];
    primaryZone = builtins.elemAt zones 0;
    secondaryZone = builtins.elemAt zones (
      if builtins.length zones > 1
      then 1
      else 0
    );
    tertiaryZone = builtins.elemAt zones (
      if builtins.length zones > 2
      then 2
      else 0
    );
    hostDomain = name: zone:
      if name == ""
      then zone
      else "${name}.${zone}";
    vpnDom = let v = tdom.vpnSubdomain or "vpn"; in hostDomain v secondaryZone;
    wgHomeLink = tlinks.wg-home or {};
    wgHomeEndpointSubdomain = wgHomeLink.endpointSubdomain or "wg";
    rawServiceHosts = builtins.listToAttrs (map (svc: {
      name = svc.name;
      value = svc.hostname or (builtins.toString svc.port);
    }) (tsvc.reverseProxyServices or []));
    aliasServiceHosts = builtins.mapAttrs (_alias: target: aliasValue rawServiceHosts target) serviceHostAliases;
  in rec {
    inherit hostDomain;
    host = name: zone: hostDomain name zone;

    tartanogluDomain = primaryZone;
    candeeDomain = secondaryZone;
    syndbDomain = tertiaryZone;

    mailDomain = primaryZone;
    mailHostname = hostDomain (tdom.mailSubdomain or "mail") primaryZone;

    vpnDomain = vpnDom;
    vpnHost = name: hostDomain name vpnDom;

    wgEndpointHost = mkWgEndpointHost wgHomeEndpointSubdomain secondaryZone;
    wireguardPort = wgHomeLink.port or 54321;

    serviceHosts = rawServiceHosts // aliasServiceHosts;

    dynamicHosts = tdom.dynamicHosts or [];
    managedZones = tdom.managedZones or [];
    codebergPagesSites = tdom.codebergPagesSites or [];
  };

  normalizeServices = {
    topology,
    domains,
  }: let
    tsvc = topology.services or {};
    tlinks = topology.links or {};
    wg = tlinks.wg-home or {};
    reverseProxyServices = map stripPklClass (tsvc.reverseProxyServices or []);
    staticFileServices = map stripPklClass (tsvc.staticFileServices or []);
    internalServices = map stripPklClass (tsvc.internalServices or []);
    reverseProxyByName = byNameFrom reverseProxyServices;
    staticFileByName = byNameFrom staticFileServices;
    internalByName = byNameFrom internalServices;
  in {
    sshPort = tsvc.sshPort or 1337;
    wireguard = {
      wgHomeEndpointHost = wg.endpointSubdomain or "wg";
      wgHomePort = wg.port or 54321;
      wgHomeDdnsHost = "wg-home.${domains.candeeDomain}";
    };
    hostSshKeyPath = tsvc.hostSshKeyPath or "/etc/ssh/id_ed25519";
    hostSshPubKeyPath = tsvc.hostSshPubKeyPath or "/etc/ssh/id_ed25519.pub";
    inherit reverseProxyServices staticFileServices internalServices;
    byName = reverseProxyByName // staticFileByName // internalByName;
    inherit reverseProxyByName staticFileByName internalByName;
    emailIdentities = stripPklClass (tsvc.emailIdentities or {});
  };

  normalizeLinks = {
    topology,
    domains ? null,
  }: let
    baseLinks = builtins.mapAttrs (linkName: link: let
      hostsOnLink =
        builtins.filter
        (name: builtins.hasAttr linkName (topology.hosts.${name}.links or {}))
        (builtins.attrNames (topology.hosts or {}));
      serverNames =
        builtins.filter
        (name: (topology.hosts.${name}.links.${linkName} or {}).role or "" == "server")
        hostsOnLink;
      serverName =
        if serverNames == []
        then null
        else builtins.head serverNames;
      clientNames =
        if serverName == null
        then []
        else builtins.filter (name: name != serverName) hostsOnLink;
    in
      link
      // {
        cidr = link.subnet or null;
        serverAddress =
          if serverName == null
          then null
          else topology.hosts.${serverName}.links.${linkName}.address;
        peers =
          builtins.map (name: let
            binding = topology.hosts.${name}.links.${linkName};
          in {
            inherit name;
            publicKey = binding.publicKey or "";
            allowedIPs = ["${binding.address}/32"];
          })
          clientNames;
      })
    (topology.links or {});
    wg = baseLinks.wg-home or {};
  in
    baseLinks
    // lib.optionalAttrs (builtins.hasAttr "wg-home" baseLinks) {
      wg-home =
        wg
        // {
          endpointHost =
            if domains == null
            then (topology.links.wg-home or {}).endpointSubdomain or "wg"
            else mkWgEndpointHost ((topology.links.wg-home or {}).endpointSubdomain or "wg") domains.candeeDomain;
        }
        // lib.optionalAttrs (domains != null) {
          ddnsHost = mkWgEndpointHost ((topology.links.wg-home or {}).endpointSubdomain or "wg") domains.candeeDomain;
        };
    };

  normalizeAll = {
    topology,
    serviceHostAliases ? {},
    directLinkNames ? ["direct-link"],
  }: let
    hosts = normalizeHosts {inherit topology directLinkNames;};
    domains = normalizeDomains {inherit topology serviceHostAliases;};
    services = normalizeServices {inherit topology domains;};
    links = normalizeLinks {inherit topology domains;};
  in {
    inherit topology hosts domains services links;
  };
}
