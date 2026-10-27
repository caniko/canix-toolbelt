# Reusable DNS record fragments.
{
  # List-returning helpers are meant to be spliced with `++`; single-record
  # helpers are meant to be inserted inline in a records list.

  hostInZone = {
    lib,
    fqdn,
    zone,
  }:
    fqdn == zone || lib.hasSuffix ".${zone}" fqdn;

  dynamicHostExcludes = {
    lib,
    zone,
    dynamicHosts,
  }: let
    dynForZone = builtins.filter (h:
      h.fqdn == zone || lib.hasSuffix ".${zone}" h.fqdn)
    dynamicHosts;
    relName = h:
      if h.fqdn == zone
      then "@"
      else lib.removeSuffix ".${zone}" h.fqdn;
  in
    builtins.concatMap (h: [
      {
        name = relName h;
        type = "A";
      }
      {
        name = relName h;
        type = "AAAA";
      }
    ])
    dynForZone;

  cloudflareDdnsProxiedExpression = {
    dynamicHosts ? null,
    proxiedFqdns ? map (h: h.fqdn) (builtins.filter (h: h.proxied) dynamicHosts),
  }: let
    proxiedNames = proxiedFqdns;
  in
    if proxiedNames == []
    then "false"
    else builtins.concatStringsSep " || " (map (n: "is(${n})") proxiedNames);

  # Drift between DDNS-owned addresses and octoDNS zone excludes. `zones` has
  # the `canix-toolbelt.dns.zones` shape (zone name -> { exclude = [{name,
  # type}]; manageRecordTypes ? null; }); the domain lists are cloudflare-ddns
  # `ip4Domains`/`ip6Domains`. Names compare case-insensitively.
  # `missingExcludes`: DDNS records octoDNS would delete on the next apply.
  # `orphanExcludes`: A/AAAA excludes no DDNS record in that zone accounts for.
  # Both empty = clean.
  ddnsExcludeDrift = {
    lib,
    zones,
    ip4Domains ? [],
    ip6Domains ? [],
  }: let
    lc = lib.toLower;
    zoneNames = builtins.attrNames zones;
    # Most specific (longest) zone owning the FQDN, or null.
    zoneFor = fqdn:
      lib.foldl' (best: zone:
        if (lc fqdn == lc zone || lib.hasSuffix ".${lc zone}" (lc fqdn)) && (best == null || builtins.stringLength zone > builtins.stringLength best)
        then zone
        else best)
      null
      zoneNames;
    relName = fqdn: zone:
      if lc fqdn == lc zone
      then "@"
      else lib.removeSuffix ".${lc zone}" (lc fqdn);
    key = zone: name: type: "${lc zone}|${lc name}|${type}";
    records = map (r: r // {zone = zoneFor r.fqdn;}) (
      map (fqdn: {
        inherit fqdn;
        type = "A";
      })
      ip4Domains
      ++ map (fqdn: {
        inherit fqdn;
        type = "AAAA";
      })
      ip6Domains
    );
    excludes = lib.concatMap (zoneName: map (e: e // {inherit zoneName;}) (zones.${zoneName}.exclude or [])) zoneNames;
    excludeKeys = map (e: key e.zoneName e.name e.type) excludes;
    recordKeys = map (r: key r.zone (relName r.fqdn r.zone) r.type) (lib.filter (r: r.zone != null) records);
    manages = zone: type: let
      types = zones.${zone}.manageRecordTypes or null;
    in
      types == null || lib.elem type types;
  in {
    missingExcludes = lib.concatMap (record: let
      inherit (record) fqdn type zone;
    in
      if zone == null
      then [
        {
          inherit fqdn type;
          reason = "no managed zone owns this FQDN";
        }
      ]
      else
        lib.optional (manages zone type && !lib.elem (key zone (relName fqdn zone) type) excludeKeys) {
          inherit fqdn type zone;
          reason = "missing ${type} exclude";
        })
    records;

    orphanExcludes = lib.filter (e: (e.type == "A" || e.type == "AAAA") && !lib.elem (key e.zoneName e.name e.type) recordKeys) excludes;
  };

  # Generate proxied/non-proxied CNAMEs to the zone apex for a list of subdomain names.
  # Used to compress the "name -> zone-apex" boilerplate in zone records.
  apexCname = {
    zone,
    names,
    proxied ? false,
    ttlAuto ? true,
  }:
    map (n: {
      name = n;
      type = "CNAME";
      data = zone;
      inherit proxied ttlAuto;
    })
    names;

  dmarc = {
    rua,
    policy ? "none",
    subdomainPolicy ? null,
    comment ? null,
  }: let
    ruaAddresses =
      if builtins.isList rua
      then rua
      else [rua];
    ruaValue = builtins.concatStringsSep "," (map (address: "mailto:${address}") ruaAddresses);
  in {
    name = "_dmarc";
    type = "TXT";
    data =
      "v=DMARC1; p=${policy}"
      + (
        if subdomainPolicy != null
        then "; sp=${subdomainPolicy}"
        else ""
      )
      + "; rua=${ruaValue}";
    ttlAuto = true;
    inherit comment;
  };

  tlsRpt = {
    rua,
    comment ? null,
  }: {
    name = "_smtp._tls";
    type = "TXT";
    data = "v=TLSRPTv1; rua=mailto:${rua}";
    ttlAuto = true;
    inherit comment;
  };

  # `id` is the policy-version identifier; bump when the policy file under
  # mta-sts.<zone>/.well-known/mta-sts.txt changes.
  mtaSts = {
    id,
    comment ? null,
  }: {
    name = "_mta-sts";
    type = "TXT";
    data = "v=STSv1; id=${id}";
    ttlAuto = true;
    inherit comment;
  };

  # Brevo (sendinblue) DKIM CNAMEs. `dkimZone` is the brevo-side zone-style
  # identifier they hand out, for example "example-com".
  brevoDkim = {dkimZone}: [
    {
      name = "brevo1._domainkey";
      type = "CNAME";
      data = "b1.${dkimZone}.dkim.brevo.com";
      ttlAuto = true;
    }
    {
      name = "brevo2._domainkey";
      type = "CNAME";
      data = "b2.${dkimZone}.dkim.brevo.com";
      ttlAuto = true;
    }
  ];

  # Cloudflare Pages publish-target alias, always proxied.
  cloudflarePagesCname = {
    name ? "@",
    pagesHost,
  }: {
    inherit name;
    type =
      if name == "@"
      then "ALIAS"
      else "CNAME";
    data = pagesHost;
    proxied = true;
    ttlAuto = true;
  };

  # Namecheap email forwarding (eforward1-5) MX + SPF.
  # DMARC is per-zone (rua addresses differ) and not included here.
  namecheapEmailForwarding = [
    {
      name = "@";
      type = "MX";
      preference = 10;
      data = "eforward1.registrar-servers.com";
      ttlAuto = true;
    }
    {
      name = "@";
      type = "MX";
      preference = 10;
      data = "eforward2.registrar-servers.com";
      ttlAuto = true;
    }
    {
      name = "@";
      type = "MX";
      preference = 10;
      data = "eforward3.registrar-servers.com";
      ttlAuto = true;
    }
    {
      name = "@";
      type = "MX";
      preference = 15;
      data = "eforward4.registrar-servers.com";
      ttlAuto = true;
    }
    {
      name = "@";
      type = "MX";
      preference = 20;
      data = "eforward5.registrar-servers.com";
      ttlAuto = true;
    }
    {
      name = "@";
      type = "TXT";
      data = "v=spf1 include:spf.efwd.registrar-servers.com ~all";
      ttlAuto = true;
    }
  ];
}
