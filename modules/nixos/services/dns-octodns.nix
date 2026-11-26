{
  config,
  inputs,
  lib,
  pkgs,
  crossbowBuildPkgs ? pkgs,
  canixCrossPackage ? (_name: package: package),
  ...
}: let
  cfg = config.canix-toolbelt.dns;
  dnsManager = inputs.dns-manager;
  dnsLib = import ../../../lib/dns.nix;
  dnsProviders = import ../../../lib/dns-providers.nix;

  inherit
    (lib)
    attrNames
    concatMap
    concatStringsSep
    elem
    filter
    filterAttrs
    foldl'
    literalExpression
    mapAttrs
    mkEnableOption
    mkOption
    types
    ;

  effectiveDnsManagerBuildPkgs =
    # dns-manager runs while rendering config into the Nix store, not on the
    # target host. Cross builds set _module.args.dnsManagerBuildPkgs so this
    # stays native to the build machine instead of compiling the renderer for
    # the target.
    config._module.args.dnsManagerBuildPkgs or crossbowBuildPkgs;
  dnsGenerate = dnsManager.lib.generate effectiveDnsManagerBuildPkgs;
  secretManagerPkg = canixCrossPackage "secret-manager" inputs.secret-manager.packages.${pkgs.stdenv.hostPlatform.system}.default;
  localSecretManagerPkg = inputs.secret-manager.packages.${effectiveDnsManagerBuildPkgs.stdenv.hostPlatform.system}.default;
  caddyLib = import ../../../lib/caddy.nix {inherit lib;};
  fleetixLib = inputs.fleetix.lib;
  networkTypes = import ../../../lib/network-types.nix {inherit lib;};

  recordTypes = ["A" "AAAA" "ALIAS" "CAA" "CNAME" "DNAME" "MX" "NS" "SOA" "SRV" "SSHFP" "TLSA" "TXT" "URI"];
  proxiableRecordTypes = ["A" "AAAA" "ALIAS" "CNAME"];

  normalizeName = name:
    if name == "@"
    then ""
    else name;
  normalizeType = type: lib.toUpper type;
  recordKey = record: "${normalizeName record.name}|${normalizeType record.type}";
  dropNulls = filterAttrs (_: value: value != null);
  valueOr = fallback: value:
    if value == null
    then fallback
    else value;
  valueWhen = condition: value:
    if condition
    then value
    else null;

  mkNullOption = type: description:
    mkOption {
      type = types.nullOr type;
      default = null;
      inherit description;
    };

  runtimeSecretPath = types.pathWith {
    absolute = true;
    inStore = false;
  };
  providerSubmodule = types.submodule ({
    config,
    name,
    ...
  }: {
    options = {
      kind = mkOption ({
          type = types.enum (attrNames dnsProviders);
          description = "DNS provider backend.";
        }
        // lib.optionalAttrs (dnsProviders ? ${name}) {default = name;});
      token = {
        secretPath = mkNullOption runtimeSecretPath "Runtime file containing the octoDNS API token.";
        agenixFile = mkNullOption types.path "Encrypted agenix token file for local octoDNS runs.";
        envName = mkOption {
          type = types.strMatching "[A-Za-z_][A-Za-z0-9_]*";
          default = dnsProviders.${config.kind}.octodnsDefaultEnv;
          defaultText = literalExpression "dnsProviders.\${kind}.octodnsDefaultEnv";
          description = "octoDNS token environment variable. Use distinct names for different credentials.";
        };
      };
      acmeTokenPath = mkNullOption runtimeSecretPath "Dedicated ACME and Stalwart credential file; defaults to token.secretPath.";
      ddnsTokenPath = mkNullOption runtimeSecretPath "Dedicated DDNS credential file; defaults to token.secretPath.";
      settings = mkOption {
        type = types.attrsOf types.anything;
        default = {};
        description = "Non-secret settings passed to the octoDNS provider.";
      };
      minTtlOverride = mkNullOption types.ints.positive "Override the provider's minimum record TTL.";
    };
  });

  recordSubmodule = types.submodule {
    options = {
      name = mkOption {
        type = types.str;
        example = "_dmarc";
        description = "Record owner name relative to the zone, or @ for the zone apex.";
      };

      type = mkOption {
        type = types.enum recordTypes;
        example = "TXT";
        description = "DNS record type.";
      };

      data = mkOption {
        type = types.nullOr types.raw;
        default = null;
        example = "v=DMARC1; p=quarantine";
        description = "Record value, or the target/exchange value for structured record types.";
      };

      dataFile = mkOption {
        type = types.nullOr (types.either types.path types.str);
        default = null;
        example = literalExpression "config.age.secrets.dns-example.path";
        description = "Runtime file containing the record value. Used for secret TXT verification values.";
      };

      dataAgenixFile = mkOption {
        type = types.nullOr (types.either types.path types.str);
        default = null;
        example = literalExpression "config.age.secrets.dns-example.rekeyFile";
        description = "Agenix encrypted source file for the record value. Used as a decryptable fallback when dataFile is not available in local planning environments.";
      };

      ttl = mkOption {
        type = types.nullOr types.int;
        default = null;
        description = "Explicit record TTL. Mutually exclusive with ttlAuto.";
      };

      ttlAuto = mkOption {
        type = types.bool;
        default = false;
        description = "Use provider automatic TTL semantics.";
      };

      proxied = mkOption {
        type = types.bool;
        default = false;
        description = "Cloudflare proxied flag. Valid only for A, AAAA, ALIAS, and CNAME records.";
      };

      comment = mkNullOption types.str "Optional provider or generated-zone comment.";
      preference = mkNullOption types.int "MX record preference.";
      exchange = mkNullOption types.str "MX record exchange host. Defaults to data when omitted.";
      priority = mkNullOption types.int "SRV or URI record priority.";
      weight = mkNullOption types.int "SRV or URI record weight.";
      port = mkNullOption types.int "SRV record port.";
      target = mkNullOption types.str "SRV or URI target. Defaults to data when omitted.";
      flags = mkNullOption types.int "CAA record flags.";
      tag = mkNullOption types.str "CAA record tag.";
      value = mkNullOption types.str "CAA record value. Defaults to data when omitted.";
      usage = mkNullOption types.int "TLSA certificate usage.";
      selector = mkNullOption types.int "TLSA selector.";
      matchingType = mkNullOption types.int "TLSA matching type.";
      certificateAssociationData = mkNullOption types.str "TLSA certificate association data.";
      algorithm = mkNullOption types.int "SSHFP algorithm.";
      fingerprintType = mkNullOption types.int "SSHFP fingerprint type.";
      fingerprint = mkNullOption types.str "SSHFP fingerprint.";
    };
  };

  redirectSubmodule = types.submodule {
    options = {
      from = mkOption {
        type = types.str;
        example = "example.com";
        description = "Hostname that should receive the HTTP redirect.";
      };

      to = mkOption {
        type = types.str;
        example = "https://www.example.com";
        description = "Absolute http(s) target URL without request URI placeholder.";
      };

      status = mkOption {
        type = types.int;
        default = 301;
        description = "HTTP 3xx status code for the redirect.";
      };

      preservePath = mkOption {
        type = types.bool;
        default = true;
        description = "Append the original request URI to the redirect target.";
      };
    };
  };

  zoneSubmodule = types.submodule {
    options = {
      provider = mkOption {
        type = types.str;
        default = "cloudflare";
        description = "ID of the authoritative DNS provider.";
      };
      migrateTo = mkNullOption types.str "Optional second octoDNS target during a migration. Does not change the authoritative provider.";

      defaultTtl = mkOption {
        type = types.int;
        default = 3600;
        description = "Default TTL applied to records in this zone unless ttl or ttlAuto is set.";
      };

      mode = mkOption {
        type = types.enum ["lenient" "strict"];
        default = "lenient";
        description = "octoDNS reconciliation mode for this zone.";
      };

      manageRecordTypes = mkOption {
        type = types.nullOr (types.listOf (types.enum recordTypes));
        default = null;
        description = "Optional record-type allowlist for octoDNS partial management.";
      };

      records = mkOption {
        type = types.listOf recordSubmodule;
        default = [];
        example = literalExpression ''
          [
            { name = "@"; type = "MX"; preference = 10; data = "mail.example.com"; }
            { name = "_dmarc"; type = "TXT"; data = "v=DMARC1; p=quarantine"; }
          ]
        '';
        description = "Records declared for this zone.";
      };

      exclude = mkOption {
        type = types.listOf (types.submodule {
          options = {
            name = mkOption {
              type = types.str;
              description = "Record owner name relative to the zone, or @ for the zone apex.";
            };
            type = mkOption {
              type = types.enum recordTypes;
              description = "Record type excluded from octoDNS management.";
            };
          };
        });
        default = [];
        description = "Records deliberately left unmanaged by octoDNS.";
      };

      tls = mkOption {
        type = types.submodule {
          options = {
            subdomain = mkOption {
              type = types.str;
              description = "Subdomain for TLS certificate configuration.";
            };
            extraTrustedNetworks = mkOption {
              type = types.listOf types.str;
              default = [];
              description = "Additional trusted networks for TLS/ACME validation.";
            };
          };
        };
        default = {};
        description = "TLS/ACME certificate configuration for this zone.";
      };
    };
  };

  zoneNames = attrNames cfg.zones;
  providerFor = id: cfg.providers.${id} or (throw "canix-toolbelt.dns: unknown provider '${id}'");
  metadataFor = id: dnsProviders.${(providerFor id).kind};

  zoneFor = fqdn:
    dnsLib.zoneForHost {
      inherit lib fqdn;
      zones = zoneNames;
    };
  credentialFor = zone: let
    id = cfg.zones.${zone}.provider;
    provider = providerFor id;
    path = valueOr provider.token.secretPath provider.acmeTokenPath;
  in
    if path == null
    then throw "canix-toolbelt.dns: '${zone}' requires providers.${id}.acmeTokenPath or token.secretPath for DNS challenges"
    else {
      inherit path;
      metadata = metadataFor id;
    };

  topologyForDnsIntents = {
    domains = {
      zones = zoneNames;
      managedZones = zoneNames;
      inherit (cfg) publicationTargets;
      pagesSites =
        map (site: {
          inherit (site) subdomain repository cnameTarget;
        })
        cfg.codebergPagesSites;
    };
    services.httpSites = config.canix-toolbelt.services.httpSites or {};
  };

  publication = import ../../../lib/dns-publication.nix {
    inherit lib fleetixLib cfg;
    topology = topologyForDnsIntents;
  };

  cnameIntentToRecord = intent: {
    name = intent.relativeName;
    type = "CNAME";
    data = intent.target;
    dataFile = null;
    dataAgenixFile = null;
    ttl = null;
    ttlAuto = true;
    inherit (intent) proxied comment;
  };

  # Private (vpn/lan) sites never get a public CNAME, even from an older
  # Fleetix helper that predates the lan scope.
  privateAccess = ["vpn" "lan"];
  managedDnsCnameIntents =
    if fleetixLib.services ? managedDnsCnameIntents
    then
      builtins.filter (intent:
        !(builtins.elem (topologyForDnsIntents.services.httpSites.${intent.name}.access or null) privateAccess))
      (fleetixLib.services.managedDnsCnameIntents {topology = topologyForDnsIntents;})
    else
      concatMap (name: let
        site = topologyForDnsIntents.services.httpSites.${name};
        zone = fleetixLib.domains.zoneForHost {
          topology = topologyForDnsIntents;
          fqdn = site.hostname;
        };
      in
        if site.dnsPublication != "managed" || builtins.elem site.access privateAccess || zone == null
        then []
        else [
          {
            inherit name zone;
            inherit (site) hostname;
            target = zone;
            proxied = site.access == "cloudflare";
            comment = null;
            source = "service";
            relativeName =
              if site.hostname == zone
              then "@"
              else lib.removeSuffix ".${zone}" site.hostname;
          }
        ])
      (attrNames topologyForDnsIntents.services.httpSites);

  synthesizedRecordsByZone =
    foldl'
    (acc: intent:
      acc
      // {
        ${intent.zone} =
          (acc.${intent.zone} or [])
          ++ [
            (cnameIntentToRecord intent)
          ];
      })
    {}
    managedDnsCnameIntents;

  # Synthesize CNAME records for Codeberg Pages sites from the topology registry.
  # Each entry in codebergPagesSites produces a CNAME from <subdomain>.<zone>
  # to <repoName>.codeberg.page.
  synthesizedCodebergPagesByZone =
    if fleetixLib.services ? pagesCnameIntents
    then
      foldl'
      (acc: intent:
        acc
        // {
          ${intent.zone} =
            (acc.${intent.zone} or [])
            ++ [
              (cnameIntentToRecord intent)
            ];
        })
      {}
      (fleetixLib.services.pagesCnameIntents {
        topology = topologyForDnsIntents;
        baseZone = cfg.codebergPagesZone;
      })
    else {};

  effectiveRecordsForZone = zoneName: zone: let
    explicitKeys = builtins.listToAttrs (
      builtins.map (record: {
        name = recordKey record;
        value = true;
      })
      zone.records
    );
    synthesized =
      lib.optional cfg.autoSynthesizeServiceCnames (synthesizedRecordsByZone.${zoneName} or [])
      ++ lib.optional cfg.autoSynthesizeCodebergPagesCnames (synthesizedCodebergPagesByZone.${zoneName} or []);
    filteredSynthesized = filter (record: !(explicitKeys.${recordKey record} or false)) (lib.flatten synthesized);
  in
    filteredSynthesized ++ (publication.recordsByZone.${zoneName} or []) ++ zone.records;

  metadataForRecord = zone: record:
    dropNulls {
      ttl = valueWhen (!record.ttlAuto) (valueOr zone.defaultTtl record.ttl);
      ttlAuto = valueWhen record.ttlAuto true;
      proxied = valueWhen ((metadataFor zone.provider).supportsProxy && elem (normalizeType record.type) proxiableRecordTypes) record.proxied;
      inherit (record) comment;
    };

  agenixRecordFor = record: {
    inherit (record) name type;
    secretPath = record.dataFile;
    agenixFile = record.dataAgenixFile;
  };

  secretPlaceholderForRecord = record: "__CANIX_DNS_SECRET_${lib.hashString "sha256" "${normalizeName record.name}|${normalizeType record.type}|${toString record.secretPath}|${toString record.agenixFile}"}__";

  secretReplacementsJson = effectiveDnsManagerBuildPkgs.writeText "cloudflare-octodns-secret-records.json" (
    builtins.toJSON (
      builtins.map (record: {
        placeholder = secretPlaceholderForRecord record;
        path =
          if record.secretPath == null
          then null
          else toString record.secretPath;
        agenixFile =
          if record.agenixFile == null
          then null
          else toString record.agenixFile;
      })
      agenixSecretRecords
    )
  );

  valueForRecord = record:
    if record.dataFile != null || record.dataAgenixFile != null
    then secretPlaceholderForRecord (agenixRecordFor record)
    else record.data;

  dataForRecord = record: let
    type = normalizeType record.type;
    data = valueForRecord record;
    structured = {
      MX = {
        inherit (record) preference;
        exchange = valueOr data record.exchange;
      };
      SRV = {
        inherit (record) priority weight port;
        target = valueOr data record.target;
      };
      URI = {
        inherit (record) priority weight;
        target = valueOr data record.target;
      };
      CAA = {
        inherit (record) flags tag;
        value = valueOr data record.value;
      };
      TLSA = {
        inherit (record) usage selector matchingType;
        certificateAssociationData = valueOr data record.certificateAssociationData;
      };
      SSHFP = {
        inherit (record) algorithm;
        type = record.fingerprintType;
        fingerprint = valueOr data record.fingerprint;
      };
    };
  in
    if (type == "TLSA" || type == "SSHFP") && builtins.isAttrs data
    then data
    else if builtins.hasAttr type structured
    then structured.${type}
    else data;

  normalizeRecord = zone: record:
    (metadataForRecord zone record)
    // {
      data = dataForRecord record;
    };

  mergeRecord = zone: acc: record: let
    name = normalizeName record.name;
    type = lib.toLower record.type;
    next = normalizeRecord zone record;
    previous = acc.${name}.${type} or null;
    asList = value:
      if builtins.isList value
      then value
      else [value];
    merged =
      if previous == null
      then next
      else
        previous
        // next
        // {
          data = (asList previous.data) ++ (asList next.data);
        };
  in
    lib.recursiveUpdate acc {
      ${name}.${type} = merged;
    };

  zoneToExtraConfig = zoneName: zone:
    foldl' (mergeRecord zone) {} (effectiveRecordsForZone zoneName zone);

  effectiveZones =
    if cfg.enable
    then cfg.zones
    else {};

  dnsConfig = {
    extraConfig = {
      defaultTTL =
        if effectiveZones == {}
        then 3600
        else (cfg.zones.${builtins.head (attrNames effectiveZones)}.defaultTtl or 3600);
      zones = mapAttrs zoneToExtraConfig effectiveZones;
    };
    inherit (cfg) redirects;
  };

  renderToken = token:
    if token.secretPath == null
    then {
      type = "env";
      name = token.envName;
    }
    else {
      type = "file";
      path = toString token.secretPath;
      inherit (token) envName;
    };

  zoneTargets = zone: [zone.provider] ++ lib.optional (zone.migrateTo != null) zone.migrateTo;
  activeIds = lib.unique (concatMap zoneTargets (lib.attrValues effectiveZones));
  activeProviders = lib.genAttrs activeIds providerFor;
  activeKinds = lib.unique (map (provider: provider.kind) (lib.attrValues activeProviders));
  legacyZones =
    mapAttrs (_: zone: {
      inherit (zone) mode manageRecordTypes;
      excludeRecords = builtins.map (record: record // {name = normalizeName record.name;}) zone.exclude;
    })
    effectiveZones;
  legacyId =
    if activeIds == []
    then "cloudflare"
    else builtins.head activeIds;
  legacyProvider = providerFor legacyId;
  legacyCloudflareOnly =
    builtins.length activeIds
    <= 1
    && builtins.all (zone: zone.migrateTo == null) (lib.attrValues effectiveZones)
    && legacyProvider.kind == "cloudflare";

  allEffectiveRecords =
    concatMap
    (zoneName:
      builtins.map
      (record: record // {_zoneName = zoneName;})
      (effectiveRecordsForZone zoneName cfg.zones.${zoneName}))
    (attrNames effectiveZones);

  secretRecordFiles = filter (record: record.dataFile != null || record.dataAgenixFile != null) allEffectiveRecords;

  agenixSecretRecords = builtins.map agenixRecordFor secretRecordFiles;

  proxiedRecordErrors =
    builtins.map
    (record: "proxied is only valid on A, AAAA, CNAME, ALIAS (${record.name} ${record.type})")
    (filter (record: record.proxied && !(elem (normalizeType record.type) proxiableRecordTypes)) allEffectiveRecords)
    ++ map (record: "${record._zoneName}: not all zone targets support proxied records (${record.name} ${record.type})")
    (filter (record: record.proxied && !(builtins.all (id: (metadataFor id).supportsProxy) (zoneTargets effectiveZones.${record._zoneName}))) allEffectiveRecords);

  ttlAutoErrors =
    builtins.map
    (record: "ttlAuto is mutually exclusive with explicit ttl (${record.name} ${record.type})")
    (filter (record: record.ttlAuto && record.ttl != null) allEffectiveRecords);

  dataFileErrors =
    builtins.map
    (record: "data is mutually exclusive with dataFile/dataAgenixFile (${record.name} ${record.type})")
    (filter (record: record.data != null && (record.dataFile != null || record.dataAgenixFile != null)) allEffectiveRecords);

  commentErrors =
    builtins.map
    (record: "DNS record comments must be at most 100 characters (${record.name} ${record.type})")
    (filter (record: record.comment != null && builtins.stringLength record.comment > 100) allEffectiveRecords);

  apexCnameErrors =
    builtins.map
    (record: "Apex (@) CNAME is invalid per RFC 1034 §3.6.2 (${record._zoneName}: ${record.name} ${record.type}). Use type = \"ALIAS\" for Cloudflare CNAME-flattening at the zone apex.")
    (filter (record: normalizeName record.name == "" && normalizeType record.type == "CNAME") allEffectiveRecords);

  redirectErrors =
    concatMap
    (redirect: let
      ctx = "redirect ${redirect.from}";
    in
      lib.optional (redirect.from == "") "${ctx}: from must not be empty"
      ++ lib.optional (!(lib.hasPrefix "https://" redirect.to || lib.hasPrefix "http://" redirect.to)) "${ctx}: to must be an absolute http(s) URL"
      ++ lib.optional (lib.hasInfix "{http.request.uri}" redirect.to) "${ctx}: to must not include {http.request.uri}; use preservePath instead"
      ++ lib.optional (!(redirect.status >= 300 && redirect.status <= 399)) "${ctx}: status must be a 3xx HTTP status code")
    cfg.redirects;

  pagesZoneErrors =
    lib.optional (cfg.autoSynthesizeCodebergPagesCnames && cfg.codebergPagesSites != [] && cfg.codebergPagesZone == null)
    "codebergPagesZone must be set explicitly when autoSynthesizeCodebergPagesCnames is enabled with a non-empty codebergPagesSites registry (no implicit first-zone fallback)"
    ++ lib.optional (cfg.autoSynthesizeCodebergPagesCnames && cfg.codebergPagesZone != null && !(builtins.elem cfg.codebergPagesZone zoneNames))
    "codebergPagesZone '${cfg.codebergPagesZone}' is not a declared zone (${toString zoneNames}); refusing to synthesize Pages records into nowhere";

  providerErrors =
    concatMap (id: lib.optional (!(cfg.providers ? ${id})) "unknown DNS provider '${id}'") activeIds
    ++ concatMap (name: let
      zone = effectiveZones.${name};
    in
      lib.optional (zone.provider == zone.migrateTo) "${name}: migrateTo must differ from provider") (attrNames effectiveZones);
  activeTokens = lib.unique (map (provider: provider.token) (lib.attrValues activeProviders));
  tokenErrors =
    lib.optional
    (builtins.length activeTokens != builtins.length (lib.unique (map (token: token.envName) activeTokens)))
    "active DNS providers with different credentials must use distinct token.envName values";
  validationErrors =
    providerErrors
    ++ (lib.optionals (providerErrors == []) (tokenErrors ++ proxiedRecordErrors))
    ++ ttlAutoErrors ++ dataFileErrors ++ commentErrors ++ apexCnameErrors ++ redirectErrors ++ pagesZoneErrors ++ publication.errors;

  validatedDnsConfig =
    if validationErrors == []
    then dnsConfig
    else throw (concatStringsSep "\n" validationErrors);

  mkOctodnsSync = {
    runtimePkgs ? pkgs,
    runtimeSecretManagerPkg ? secretManagerPkg,
    cache ? {
      enable = false;
      dir = null;
    },
  }: let
    cacheEnabled = cache.enable or false;
    cacheDir = cache.dir or null;
    runtimeDnsGenerate = dnsManager.lib.generate runtimePkgs;
    dropIgnoredPkg = runtimePkgs.python3Packages.toPythonModule (
      runtimePkgs.runCommand "canix-octodns-drop-ignored" {} ''
        mkdir -p "$out/${runtimePkgs.python3.sitePackages}"
        cp ${./dns-octodns-drop-ignored.py} "$out/${runtimePkgs.python3.sitePackages}/canix_octodns_drop_ignored.py"
      ''
    );
    octodnsEnv = runtimePkgs.python3.withPackages (_:
      [runtimePkgs.octodns dropIgnoredPkg]
      ++ map (
        kind: let
          attr = dnsProviders.${kind}.octodnsProviderAttr;
        in
          runtimePkgs.octodns-providers.${attr}
        or runtimeDnsGenerate.octodnsProviders.${attr}
        or (throw "canix-toolbelt.dns: no octoDNS package available for '${kind}'")
      )
      activeKinds);
    validatedCacheDir =
      if !cacheEnabled
      then null
      else if cacheDir == null
      then throw "canix-toolbelt.dns.localCache.dir is required when localCache.enable = true"
      else if !builtins.isString cacheDir
      then throw "canix-toolbelt.dns.localCache.dir must be a runtime string path, not a Nix path"
      else if lib.hasPrefix "/nix/store/" cacheDir
      then throw "canix-toolbelt.dns.localCache.dir must not point into /nix/store"
      else cacheDir;
    defaultAgeIdentities = concatStringsSep ":" (builtins.map toString cfg.agenix.identityPaths);
  in
    runtimePkgs.writeShellScript "octodns-sync" ''
      set -euo pipefail
      if [ "$#" -lt 1 ]; then
        echo "usage: octodns-sync <config-dir> [octodns-sync args...]" >&2
        exit 2
      fi
      source_config="$1"
      shift

      workdir="$(${runtimePkgs.coreutils}/bin/mktemp -d)"
      trap '${runtimePkgs.coreutils}/bin/rm -rf "$workdir"' EXIT
      ${runtimePkgs.coreutils}/bin/cp -RL "$source_config"/. "$workdir/config"
      ${runtimePkgs.coreutils}/bin/chmod -R u+w "$workdir/config"
      export SECRET_MANAGER_BIN=${runtimeSecretManagerPkg}/bin/secret-manager
      export CANIX_DNS_AGE_IDENTITIES="''${CANIX_DNS_AGE_IDENTITIES:-${defaultAgeIdentities}}"
      ${lib.optionalString cacheEnabled ''
        export CANIX_DNS_DECRYPT_CACHE_DIR="${validatedCacheDir}"
      ''}
      ${runtimePkgs.python3}/bin/python ${./dns-octodns-cloudflare-substitute-secrets.py} \
        "$workdir/config" "$source_config/zones" "$workdir/config/zones" \
        - ${secretReplacementsJson} "$@"

      load_token() {
        local name="$1" path="$2" agenix="$3" file_var="''${1}_FILE" token
        if [ -n "''${!name:-}" ]; then
          export "$name"
          return
        elif [ -n "''${CREDENTIALS_DIRECTORY:-}" ] && [ -r "$CREDENTIALS_DIRECTORY/$name" ]; then
          token="$(${runtimePkgs.coreutils}/bin/cat "$CREDENTIALS_DIRECTORY/$name")"
        elif [ -r "''${!file_var:-}" ]; then
          token="$(${runtimePkgs.coreutils}/bin/cat "''${!file_var}")"
        elif [ -r "$path" ]; then
          token="$(${runtimePkgs.coreutils}/bin/cat "$path")"
        elif [ -r "$agenix" ]; then
          token="$(${runtimePkgs.python3}/bin/python ${./dns-octodns-cloudflare-substitute-secrets.py} --decrypt-file "$agenix")"
        else
          echo "$name is unset and no readable token file is available" >&2
          echo "set CANIX_DNS_AGE_IDENTITIES to colon-separated age/ssh identity paths to decrypt agenix sources locally" >&2
          return 1
        fi
        if [ -z "$token" ]; then
          echo "$name token file is empty" >&2
          return 1
        fi
        export "$name=$token"
      }

      ${lib.concatMapStringsSep "\n" (token: "load_token ${lib.escapeShellArgs [
          token.envName
          (toString token.secretPath)
          (toString token.agenixFile)
        ]}")
        activeTokens}

      ${octodnsEnv}/bin/octodns-sync --config-file "$workdir/config/config.yaml" "$@"
    '';

  octodnsSync = mkOctodnsSync {};

  mkOctodnsConfig = builtins.seq validatedDnsConfig (
    if dnsGenerate ? sync
    then
      dnsGenerate.sync {
        dnsConfig = validatedDnsConfig;
        extraGlobalConfig.processors.drop-ignored.class = "canix_octodns_drop_ignored.DropIgnoredRecordsProcessor";
        zones = mapAttrs (name: zone:
          legacyZones.${name}
          // {
            targets = zoneTargets zone;
            processors = ["drop-ignored"];
          })
        effectiveZones;
        providers = mapAttrs (_: provider:
          {inherit (provider) kind settings;}
          // {token = renderToken provider.token;}
          // lib.optionalAttrs (provider.minTtlOverride != null) {inherit (provider) minTtlOverride;})
        activeProviders;
      }
    else if legacyCloudflareOnly
    then
      dnsGenerate.cloudflare {
        dnsConfig = validatedDnsConfig;
        token = renderToken legacyProvider.token;
        zones = legacyZones;
        extraProviderSettings =
          legacyProvider.settings
          // lib.optionalAttrs (legacyProvider.minTtlOverride != null) {min_ttl = legacyProvider.minTtlOverride;};
      }
    else throw "canix-toolbelt.dns: these zone targets require a dns-manager pin exposing lib.generate.sync"
  );

  octodnsConfig = mkOctodnsConfig;
  caddyRedirectRoutes = builtins.map caddyLib.mkRedirectRoute cfg.redirects;

  localSync = mkOctodnsSync {
    runtimePkgs = effectiveDnsManagerBuildPkgs;
    runtimeSecretManagerPkg = localSecretManagerPkg;
    cache = cfg.localCache;
  };
  octodnsConfigLocal = effectiveDnsManagerBuildPkgs.linkFarm "octodns-local" [
    {
      name = "config.yaml";
      path = "${mkOctodnsConfig}/config.yaml";
    }
    {
      name = "zones";
      path = "${mkOctodnsConfig}/zones";
    }
    {
      name = "octodns-sync";
      path = localSync;
    }
    {
      name = "octodns-sync-cloudflare";
      path = localSync;
    }
  ];

  reconcilerEnabled =
    cfg.enable
    && cfg.reconciler.enable
    && activeTokens != []
    && builtins.all (token: token.secretPath != null) activeTokens;
  applyExtraArgs =
    concatStringsSep " "
    (["--doit"] ++ lib.optional cfg.reconciler.applyForce "--force" ++ cfg.reconciler.extraApplyArgs);

  mkReconcileService = {
    description,
    extraArgs,
  }: {
    inherit description;
    after = ["network-online.target"];
    wants = ["network-online.target"];
    serviceConfig = {
      Type = "oneshot";
      User = cfg.reconciler.user;
      Group = cfg.reconciler.user;
      LoadCredential = map (token: "${token.envName}:${token.secretPath}") activeTokens;
      ExecStart = "${octodnsSync} ${cfg.octodnsConfig} ${extraArgs}";
    };
  };
in {
  imports = [
    ./caddy-base.nix
    ./dns-ddns.nix
    (lib.mkRenamedOptionModule
      ["canix-toolbelt" "dns" "cloudflareToken"]
      ["canix-toolbelt" "dns" "providers" "cloudflare" "token"])
  ];

  options.canix-toolbelt.dns = {
    enable = mkEnableOption "declarative DNS zones via dns-manager";

    zones = mkOption {
      type = types.attrsOf zoneSubmodule;
      default = {};
      description = "DNS zones declared on this host.";
    };

    publicationTargets = mkOption {
      type = types.attrsOf (types.submodule {
        options = {
          hostname = mkOption {
            type = networkTypes.hostname;
            description = "Static destination hostname in a managed zone.";
          };
          targetHost = mkOption {
            type = types.str;
            description = "Fleet host serving this destination; validated by Fleetix.";
          };
          ipv4 = mkOption {
            type = networkTypes.ipv4;
            description = "Explicit desired public IPv4, promoted into authored topology.";
          };
          ipv6 = mkOption {
            type = types.nullOr networkTypes.ipv6;
            default = null;
            description = "Optional desired IPv6; null means no published AAAA.";
          };
        };
      });
      default = {};
      description = "Fleetix static publication targets. Empty preserves legacy DNS synthesis. Each target owns A/AAAA, even when IPv6 is absent.";
    };

    redirects = mkOption {
      type = types.listOf redirectSubmodule;
      default = [];
      description = "HTTP redirect intents projected to Caddy routes without invoking the dns-manager renderer.";
    };

    redirectIngress = mkOption {
      type = types.str;
      default = "public";
      description = "Caddy server that receives DNS redirect routes.";
    };

    providers = mkOption {
      type = types.attrsOf providerSubmodule;
      default = {};
      description = "DNS provider instances keyed by ID.";
    };

    acmeFor = mkOption {
      type = types.functionTo types.attrs;
      readOnly = true;
      description = "Return NixOS ACME DNS-01 settings for the most specific managed zone matching a hostname.";
    };

    stalwartDnsFor = mkOption {
      type = types.functionTo types.attrs;
      readOnly = true;
      description = "Return the Stalwart DNS provider type and credential file for a declared zone.";
    };

    autoSynthesizeServiceCnames = mkOption {
      type = types.bool;
      default = true;
      description = "Synthesize Cloudflare CNAME records from canix-toolbelt service registry hostnames in declared zones.";
    };

    autoSynthesizeCodebergPagesCnames = mkOption {
      type = types.bool;
      default = true;
      description = "Synthesize CNAME records for Codeberg Pages sites from the codebergPagesSites registry.";
    };

    codebergPagesZone = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Zone under which Codeberg Pages CNAME records are synthesized. Must be set explicitly when autoSynthesizeCodebergPagesCnames is enabled with a non-empty registry; evaluation fails otherwise instead of guessing the first declared zone.";
    };

    codebergPagesSites = mkOption {
      type = types.listOf (types.submodule {
        freeformType = types.attrsOf types.raw;
        options = {
          __pkl_class = mkOption {
            type = types.nullOr types.str;
            default = null;
            description = "Pkl class name preserved by pklx serializer. Ignored by the DNS module.";
          };
          subdomain = mkOption {
            type = types.str;
            description = "Subdomain for the Pages site (e.g. 'myproject' for myproject.example.com).";
          };
          repository = mkOption {
            type = types.str;
            description = "Repository associated with the Pages site (e.g. 'example/myproject'). Informational; the record target below is authoritative.";
          };
          cnameTarget = mkOption {
            type = types.str;
            description = "Explicit DNS CNAME target for the Pages provider (e.g. 'example.codeberg.page'). Never derived from the repository owner.";
          };
        };
      });
      default = [];
      description = "Registry of project Pages sites. Each entry generates a CNAME from <subdomain>.<zone> to the explicit cnameTarget.";
    };

    reconciler = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Run octoDNS reconciliation services on this host. Disable when reconciliation is performed from a local flake app.";
      };

      user = mkOption {
        type = types.str;
        default = "cloudflare-octodns";
        description = "System user that runs the octoDNS reconcile services.";
      };

      applyForce = mkOption {
        type = types.bool;
        default = false;
        description = "Pass --force to octoDNS for the activation apply service.";
      };

      extraApplyArgs = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Additional octoDNS arguments passed only to the activation apply service.";
      };
    };

    agenix = {
      records = mkOption {
        type = types.listOf types.attrs;
        readOnly = true;
        description = "Agenix-backed DNS record secret descriptors consumed by the toolbelt octoDNS wrapper.";
      };

      identityPaths = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Default local age identity paths used when decrypting DNS agenix sources outside the target host.";
      };
    };

    localCache = {
      enable = mkOption {
        type = types.bool;
        default = false;
        description = "Enable a runtime plaintext cache for local DNS agenix decryptions.";
      };

      dir = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Runtime cache directory for local DNS agenix decryptions. Must not point into the Nix store.";
      };
    };

    dnsConfig = mkOption {
      type = types.raw;
      readOnly = true;
      description = "dns-manager input generated from canix-toolbelt.dns.zones.";
    };

    octodnsConfig = mkOption {
      type = types.package;
      readOnly = true;
      description = "Generated octoDNS configuration directory.";
    };

    octodnsConfigLocal = mkOption {
      type = types.package;
      readOnly = true;
      description = "Generated octoDNS configuration directory with the local planning wrapper.";
    };

    cloudflareZoneExcludes = mkOption {
      type = types.raw;
      readOnly = true;
      description = "Cloudflare zone exclude records after provider-side name normalization.";
    };
  };

  config = {
    assertions =
      builtins.map (message: {
        assertion = false;
        inherit message;
      })
      validationErrors;

    canix-toolbelt.dns = {
      providers.cloudflare.kind = lib.mkDefault "cloudflare";
      dnsConfig = validatedDnsConfig;
      cloudflareZoneExcludes =
        mapAttrs (_: zone: zone.excludeRecords)
        (filterAttrs (name: _: (metadataFor effectiveZones.${name}.provider).supportsProxy) legacyZones);
      acmeFor = fqdn: let
        zone = zoneFor fqdn;
        credential = credentialFor zone;
      in
        if zone == null
        then throw "canix-toolbelt.dns: no declared zone matches '${fqdn}'"
        else {
          dnsProvider = credential.metadata.legoProvider;
          credentialFiles.${credential.metadata.legoEnvFileVar} = credential.path;
        };
      stalwartDnsFor = zone: let
        matched = zoneFor zone;
        credential = credentialFor matched;
      in
        if matched == null || dnsLib.normalizeHostname lib matched != dnsLib.normalizeHostname lib zone
        then throw "canix-toolbelt.dns: unknown Stalwart DNS zone '${zone}'"
        else {
          type = credential.metadata.stalwartType;
          tokenPath = credential.path;
        };
      inherit octodnsConfig;
      inherit octodnsConfigLocal;
      agenix.records = agenixSecretRecords;
    };

    canix-toolbelt.services.caddy.servers.${cfg.redirectIngress}.routes = lib.mkAfter caddyRedirectRoutes;

    users.groups = lib.mkIf reconcilerEnabled {
      ${cfg.reconciler.user} = {};
    };

    users.users = lib.mkIf reconcilerEnabled {
      ${cfg.reconciler.user} = {
        isSystemUser = true;
        group = cfg.reconciler.user;
        description = "octoDNS reconciler";
      };
    };

    systemd.services = lib.mkIf reconcilerEnabled {
      cloudflare-octodns = mkReconcileService {
        description = "DNS reconcile (plan / dry-run)";
        extraArgs = "";
      };
      cloudflare-octodns-apply =
        (mkReconcileService {
          description = "DNS reconcile (apply changes)";
          extraArgs = applyExtraArgs;
        })
        // {
          wantedBy = ["multi-user.target"];
          restartTriggers = [cfg.octodnsConfig];
          unitConfig.ConditionPathExists = map (token: toString token.secretPath) activeTokens;
        };
    };
  };
}
