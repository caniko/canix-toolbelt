{
  config,
  inputs,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.dns;
  nixosDns = inputs.nixos-dns;
  cloudflareUtils = nixosDns.utils.cloudflare;

  inherit
    (lib)
    attrNames
    concatMap
    concatStringsSep
    elem
    filter
    filterAttrs
    foldl'
    hasSuffix
    literalExpression
    mapAttrs
    mkEnableOption
    mkOption
    removeSuffix
    splitString
    types
    ;

  dnsGenerate = nixosDns.utils.generate pkgs;

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

  zoneSubmodule = types.submodule {
    options = {
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
    };
  };

  allServices =
    (config.canix-toolbelt.services.reverseProxyServices or [])
    ++ (config.canix-toolbelt.services.staticFileServices or []);

  zoneNames = attrNames cfg.zones;

  serviceZone = hostname: let
    matches = filter (zone: hostname == zone || hasSuffix ".${zone}" hostname) zoneNames;
  in
    if matches == []
    then null
    else
      foldl'
      (best: zone:
        if best == null || builtins.length (splitString "." zone) > builtins.length (splitString "." best)
        then zone
        else best)
      null
      matches;

  serviceRelativeName = zone: hostname:
    if hostname == zone
    then "@"
    else removeSuffix ".${zone}" hostname;

  synthesizedRecordsByZone =
    foldl'
    (acc: service: let
      zone = serviceZone service.hostname;
    in
      if zone == null || (service.vpnOnly or false) || !(service.publishCname or true)
      then acc
      else
        acc
        // {
          ${zone} =
            (acc.${zone} or [])
            ++ [
              {
                name = serviceRelativeName zone service.hostname;
                type = "CNAME";
                data = zone;
                dataFile = null;
                dataAgenixFile = null;
                ttl = null;
                ttlAuto = true;
                proxied = service.cloudflareProxied or false;
                comment = service.dnsComment or null;
              }
            ];
        })
    {}
    allServices;

  effectiveRecordsForZone = zoneName: zone: let
    explicitKeys = builtins.listToAttrs (
      builtins.map (record: {
        name = recordKey record;
        value = true;
      })
      zone.records
    );
    synthesized =
      if cfg.autoSynthesizeServiceCnames
      then synthesizedRecordsByZone.${zoneName} or []
      else [];
    filteredSynthesized = filter (record: !(explicitKeys.${recordKey record} or false)) synthesized;
  in
    filteredSynthesized ++ zone.records;

  metadataForRecord = zone: record:
    dropNulls {
      ttl = valueWhen (!record.ttlAuto) (valueOr zone.defaultTtl record.ttl);
      ttlAuto = valueWhen record.ttlAuto true;
      proxied = valueWhen (elem (normalizeType record.type) proxiableRecordTypes) record.proxied;
      inherit (record) comment;
    };

  agenixRecordFor = record: {
    inherit (record) name type;
    secretPath = record.dataFile;
    agenixFile = record.dataAgenixFile;
  };

  valueForRecord = record:
    if record.dataFile != null || record.dataAgenixFile != null
    then cloudflareUtils.secretPlaceholderForRecord (agenixRecordFor record)
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
  };

  token =
    if cfg.cloudflareToken.secretPath == null
    then {
      type = "env";
      name = cfg.cloudflareToken.envName;
    }
    else {
      type = "file";
      path = toString cfg.cloudflareToken.secretPath;
      inherit (cfg.cloudflareToken) envName;
    };

  cloudflareZones =
    mapAttrs (_: zone: {
      inherit (zone) mode manageRecordTypes;
      excludeRecords = builtins.map (record: record // {name = normalizeName record.name;}) zone.exclude;
    })
    effectiveZones;

  allEffectiveRecords = concatMap (zoneName: effectiveRecordsForZone zoneName cfg.zones.${zoneName}) (attrNames effectiveZones);

  secretRecordFiles = filter (record: record.dataFile != null || record.dataAgenixFile != null) allEffectiveRecords;

  agenixSecretRecords = builtins.map agenixRecordFor secretRecordFiles;

  proxiedRecordErrors =
    builtins.map
    (record: "proxied is only valid on A, AAAA, CNAME, ALIAS (${record.name} ${record.type})")
    (filter (record: record.proxied && !(elem (normalizeType record.type) proxiableRecordTypes)) allEffectiveRecords);

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

  validationErrors = proxiedRecordErrors ++ ttlAutoErrors ++ dataFileErrors ++ commentErrors;

  validatedDnsConfig =
    if validationErrors == []
    then dnsConfig
    else throw (concatStringsSep "\n" validationErrors);

  octodnsSync = cloudflareUtils.mkSyncWrapper pkgs {
    substitutionScript = cloudflareUtils.mkSubstitutionScript pkgs {
      records = agenixSecretRecords;
      extraEnvIdentities = config.services.canixDns.agenix.identityPaths;
    };
    inherit (cfg) cloudflareToken;
    extraEnvIdentities = config.services.canixDns.agenix.identityPaths;
  };

  octodnsConfig = dnsGenerate.cloudflareConfig {
    dnsConfig = validatedDnsConfig;
    inherit token;
    zones = cloudflareZones;
    agenix = {
      inherit (cfg) cloudflareToken;
      records = agenixSecretRecords;
      inherit (config.services.canixDns.agenix) identityPaths;
    };
  };

  reconcilerEnabled = cfg.enable && cfg.cloudflareToken.secretPath != null;

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
      Environment = "CLOUDFLARE_API_TOKEN_FILE=${cfg.cloudflareToken.secretPath}";
      ExecStart = "${octodnsSync} ${cfg.octodnsConfig} ${extraArgs}";
    };
  };
in {
  imports = [
    nixosDns.nixosModules.dns-secrets
  ];

  options.canix-toolbelt.dns = {
    enable = mkEnableOption "declarative DNS zones via NixOS-DNS";

    zones = mkOption {
      type = types.attrsOf zoneSubmodule;
      default = {};
      description = "DNS zones declared on this host.";
    };

    cloudflareToken = {
      secretPath = mkOption {
        type = types.nullOr types.path;
        default = null;
        description = "Optional path to an agenix-mounted file containing the Cloudflare API token.";
      };

      agenixFile = mkOption {
        type = types.nullOr types.path;
        default = null;
        description = "Optional encrypted agenix source file for the Cloudflare API token. Used as a decryptable fallback when secretPath is not mounted.";
      };

      envName = mkOption {
        type = types.str;
        default = "CLOUDFLARE_API_TOKEN";
        description = "Environment variable exposed to octoDNS for Cloudflare authentication.";
      };
    };

    autoSynthesizeServiceCnames = mkOption {
      type = types.bool;
      default = true;
      description = "Synthesize Cloudflare CNAME records from canix-toolbelt service registry hostnames in declared zones.";
    };

    reconciler = {
      user = mkOption {
        type = types.str;
        default = "cloudflare-octodns";
        description = "System user that runs the octoDNS reconcile services.";
      };
    };

    dnsConfig = mkOption {
      type = types.raw;
      readOnly = true;
      description = "NixOS-DNS input generated from canix-toolbelt.dns.zones.";
    };

    octodnsConfig = mkOption {
      type = types.package;
      readOnly = true;
      description = "Generated octoDNS Cloudflare configuration directory.";
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
      dnsConfig = validatedDnsConfig;
      inherit octodnsConfig;
    };

    services.canixDns.agenix = lib.mkIf cfg.enable {
      enable = cfg.cloudflareToken.secretPath != null || cfg.cloudflareToken.agenixFile != null || secretRecordFiles != [];
      inherit (cfg) cloudflareToken;
      records = agenixSecretRecords;
    };

    users.groups = lib.mkIf reconcilerEnabled {
      ${cfg.reconciler.user} = {};
    };

    users.users = lib.mkIf reconcilerEnabled {
      ${cfg.reconciler.user} = {
        isSystemUser = true;
        group = cfg.reconciler.user;
        description = "octoDNS Cloudflare reconciler";
      };
    };

    systemd.services = lib.mkIf reconcilerEnabled {
      cloudflare-octodns = mkReconcileService {
        description = "Cloudflare DNS reconcile (plan / dry-run)";
        extraArgs = "";
      };
      cloudflare-octodns-apply =
        (mkReconcileService {
          description = "Cloudflare DNS reconcile (apply changes)";
          extraArgs = "--doit";
        })
        // {
          wantedBy = ["multi-user.target"];
          restartTriggers = [cfg.octodnsConfig];
          unitConfig.ConditionPathExists = toString cfg.cloudflareToken.secretPath;
        };
    };
  };
}
