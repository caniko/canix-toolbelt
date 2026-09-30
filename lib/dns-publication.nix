# Static-publication rendering and writer ownership checks for the DNS adapter.
{
  lib,
  fleetixLib,
  cfg,
  topology,
}: let
  inherit (lib) concatMap elem filter foldl';
  normalizeName = name:
    if name == "@"
    then ""
    else lib.toLower name;
  addressIntents =
    if fleetixLib.domains ? publicationAddressIntents
    then fleetixLib.domains.publicationAddressIntents {inherit topology;}
    else if cfg.publicationTargets == {}
    then []
    else throw "canix-toolbelt.dns: explicit publication requires a Fleetix input with publicationAddressIntents";
  recordsByZone = foldl' (acc: intent:
    acc
    // {
      ${intent.zone} =
        (acc.${intent.zone} or [])
        ++ map (record:
          {
            name = intent.relativeName;
            dataFile = null;
            dataAgenixFile = null;
            ttl = null;
            ttlAuto = true;
            proxied = false;
            comment = "Explicit Fleetix publication destination";
          }
          // record) ([
            {
              type = "A";
              data = intent.ipv4;
            }
          ]
          ++ lib.optional (intent.ipv6 != null) {
            type = "AAAA";
            data = intent.ipv6;
          });
    }) {}
  addressIntents;
  sites = lib.attrValues (topology.services.httpSites or {});
  publishedSites = filter (site: (site.publicationTarget or null) != null) sites;
  staticNames = lib.unique (map lib.toLower (
    map (target: target.hostname) (lib.attrValues cfg.publicationTargets)
    ++ map (site: site.hostname) publishedSites
  ));
  errors =
    concatMap (site:
      lib.optional (!(builtins.hasAttr site.publicationTarget cfg.publicationTargets)) "publication ${site.hostname}: unknown destination ${site.publicationTarget}"
      ++ lib.optional (site.access != "direct" || site.dnsPublication != "managed") "publication ${site.hostname}: requires managed DNS-only public access"
      ++ lib.optional (!cfg.autoSynthesizeServiceCnames) "publication ${site.hostname}: automatic service DNS synthesis must be enabled")
    publishedSites
    ++ concatMap (hostname: let
      zoneName = fleetixLib.domains.zoneForHost {
        inherit topology;
        fqdn = hostname;
      };
      zone =
        if zoneName == null
        then {}
        else cfg.zones.${zoneName};
      owner =
        if zoneName == null
        then hostname
        else if hostname == zoneName
        then ""
        else lib.removeSuffix ".${zoneName}" hostname;
      conflicts = record: normalizeName record.name == owner && elem (lib.toUpper record.type) ["A" "AAAA" "CNAME"];
      pagesOwned = cfg.autoSynthesizeCodebergPagesCnames && cfg.codebergPagesZone != null && builtins.any (site: lib.toLower "${site.subdomain}.${cfg.codebergPagesZone}" == hostname) cfg.codebergPagesSites;
    in
      lib.optional (zoneName == null) "publication ${hostname}: destination must belong to a declared managed zone"
      ++ lib.optional (builtins.any conflicts ((zone.records or []) ++ (zone.exclude or []))) "publication ${hostname}: explicit/excluded A, AAAA or CNAME ownership must be removed before static publication"
      ++ lib.optional pagesOwned "publication ${hostname}: Pages owns this hostname"
      ++ lib.optional ((zone.manageRecordTypes or null) != null && !(builtins.all (type: elem type zone.manageRecordTypes) ["A" "AAAA" "CNAME"])) "publication ${hostname}: DNS management must own A, AAAA and CNAME, including removal of absent AAAA")
    staticNames
    ++ concatMap (site: let
      matches = filter (target: lib.toLower target.hostname == lib.toLower site.hostname) (lib.attrValues cfg.publicationTargets);
      destination = site.publicationTarget or null;
      selected =
        if destination == null
        then null
        else cfg.publicationTargets.${destination} or null;
    in
      lib.optional (site.dnsPublication == "managed" && matches != [] && (selected == null || lib.toLower selected.hostname != lib.toLower site.hostname)) "publication ${site.hostname}: destination hostname cannot also have a CNAME to another target")
    sites;
in {inherit recordsByZone errors;}
