{
  config,
  inputs,
  lib,
  pkgs,
  crossbowBuildPkgs ? pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.dns;
  dnsManager = inputs.dns-manager;
  secretManagerPkg = inputs.secret-manager.packages.${pkgs.stdenv.hostPlatform.system}.default;

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

  dnsGenerate = dnsManager.lib.generate crossbowBuildPkgs;

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

  # Synthesize CNAME records for Codeberg Pages sites from the topology registry.
  # Each entry in codebergPagesSites produces a CNAME from <subdomain>.tartanoglu.com
  # to <repoName>.caniko.codeberg.page.
  synthesizedCodebergPagesByZone = let
    pagesTarget = site: let
      parts = splitString "/" site.targetRepo;
      repoName = builtins.elemAt parts (builtins.length parts - 1);
    in "${repoName}.caniko.codeberg.page";
  in
    foldl'
    (acc: site: let
      zone = serviceZone "${site.subdomain}.tartanoglu.com";
    in
      if zone == null
      then acc
      else
        acc
        // {
          ${zone} =
            (acc.${zone} or [])
            ++ [
              {
                name = site.subdomain;
                type = "CNAME";
                data = pagesTarget site;
                dataFile = null;
                dataAgenixFile = null;
                ttl = null;
                ttlAuto = true;
                proxied = false;
                comment = "Codeberg Pages: ${site.targetRepo}";
              }
            ];
        })
    {}
    (cfg.codebergPagesSites or []);

  effectiveRecordsForZone = zoneName: zone: let
    explicitKeys = builtins.listToAttrs (
      builtins.map (record: {
        name = recordKey record;
        value = true;
      })
      zone.records
    );
    synthesized =
      lib.optional (cfg.autoSynthesizeServiceCnames) (synthesizedRecordsByZone.${zoneName} or [])
      ++ lib.optional (cfg.autoSynthesizeCodebergPagesCnames) (synthesizedCodebergPagesByZone.${zoneName} or []);
    filteredSynthesized = filter (record: !(explicitKeys.${recordKey record} or false)) (lib.flatten synthesized);
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

  secretPlaceholderForRecord = record: "__CANIX_DNS_SECRET_${lib.hashString "sha256" "${normalizeName record.name}|${normalizeType record.type}|${toString record.secretPath}|${toString record.agenixFile}"}__";

  secretReplacementsJson = pkgs.writeText "cloudflare-octodns-secret-records.json" (
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

  apexCnameErrors =
    builtins.map
    (record: "Apex (@) CNAME is invalid per RFC 1034 §3.6.2 (${record._zoneName}: ${record.name} ${record.type}). Use type = \"ALIAS\" for Cloudflare CNAME-flattening at the zone apex.")
    (filter (record: normalizeName record.name == "" && normalizeType record.type == "CNAME") allEffectiveRecords);

  validationErrors = proxiedRecordErrors ++ ttlAutoErrors ++ dataFileErrors ++ commentErrors ++ apexCnameErrors;

  validatedDnsConfig =
    if validationErrors == []
    then dnsConfig
    else throw (concatStringsSep "\n" validationErrors);

  mkOctodnsSync = {
    cache ? {
      enable = false;
      dir = null;
    },
  }: let
    cacheEnabled = cache.enable or false;
    cacheDir = cache.dir or null;
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
    pkgs.writeShellScript "cloudflare-octodns-sync" ''
      set -euo pipefail
      if [ "$#" -lt 1 ]; then
        echo "usage: cloudflare-octodns-sync <config-dir> [octodns-sync args...]" >&2
        exit 2
      fi
      source_config="$1"
      shift

      workdir="$(${pkgs.coreutils}/bin/mktemp -d)"
      trap '${pkgs.coreutils}/bin/rm -rf "$workdir"' EXIT
      ${pkgs.coreutils}/bin/cp -RL "$source_config"/. "$workdir/config"
      ${pkgs.coreutils}/bin/chmod -R u+w "$workdir/config"
      export SECRET_MANAGER_BIN=${secretManagerPkg}/bin/secret-manager
      export CANIX_DNS_AGE_IDENTITIES="''${CANIX_DNS_AGE_IDENTITIES:-${defaultAgeIdentities}}"
      ${lib.optionalString cacheEnabled ''
        export CANIX_DNS_DECRYPT_CACHE_DIR="${validatedCacheDir}"
      ''}
      ${pkgs.python3}/bin/python ${./dns-octodns-cloudflare-substitute-secrets.py} \
        "$workdir/config" "$source_config/zones" "$workdir/config/zones" \
        - ${secretReplacementsJson} "$@"

      decrypt_agenix_file() {
        encrypted_file="$1"
        ${lib.optionalString cacheEnabled ''
        cache_dir="${validatedCacheDir}"
        ${pkgs.coreutils}/bin/mkdir -p "$cache_dir"
        ${pkgs.coreutils}/bin/chmod 0700 "$cache_dir" || {
          echo "failed to set cache dir perms to 0700: $cache_dir" >&2
          return 1
        }
        cache_dir_mode="$(${pkgs.coreutils}/bin/stat -c %a "$cache_dir")" || return 1
        if [ "$cache_dir_mode" != 700 ]; then
          echo "cache dir perms not 0700: $cache_dir" >&2
          return 1
        fi
        cache_key="$(${pkgs.coreutils}/bin/sha256sum "$encrypted_file" | ${pkgs.coreutils}/bin/cut -d' ' -f1)"
        cache_file="$cache_dir/$cache_key"
        if [ -r "$cache_file" ]; then
          cache_file_mode="$(${pkgs.coreutils}/bin/stat -c %a "$cache_file")" || return 1
          if [ "$cache_file_mode" != 600 ]; then
            echo "cache file perms not 0600: $cache_file" >&2
            return 1
          fi
          ${pkgs.coreutils}/bin/cat "$cache_file"
          return 0
        fi
      ''}
        # Build --identity flags from CANIX_DNS_AGE_IDENTITIES
        if [ -n "$CANIX_DNS_AGE_IDENTITIES" ]; then
          old_ifs="$IFS"
          IFS=:
          set -- $CANIX_DNS_AGE_IDENTITIES
          IFS="$old_ifs"
          identity_args=""
          for identity in "$@"; do
            identity_args="$identity_args --identity $identity"
          done
          plaintext="$(${secretManagerPkg}/bin/secret-manager decrypt $identity_args "$encrypted_file")" || return 1
        else
          plaintext="$(${secretManagerPkg}/bin/secret-manager decrypt "$encrypted_file")" || return 1
        fi
        ${lib.optionalString cacheEnabled ''
        tmp="$(${pkgs.coreutils}/bin/mktemp "$cache_dir/.tmp.XXXXXX")" || return 1
        cleanup_tmp() {
          ${pkgs.coreutils}/bin/rm -f "$tmp"
        }
        trap cleanup_tmp RETURN
        ${pkgs.coreutils}/bin/chmod 0600 "$tmp" || return 1
        printf '%s' "$plaintext" > "$tmp" || return 1
        ${pkgs.coreutils}/bin/sync -f "$tmp" || return 1
        ${pkgs.coreutils}/bin/mv -f "$tmp" "$cache_file" || return 1
        trap - RETURN
        ${pkgs.coreutils}/bin/chmod 0600 "$cache_file" || return 1
        cache_file_mode="$(${pkgs.coreutils}/bin/stat -c %a "$cache_file")" || return 1
        if [ "$cache_file_mode" != 600 ]; then
          echo "cache file perms not 0600: $cache_file" >&2
          return 1
        fi
      ''}
        printf '%s' "$plaintext"
        return 0
      }

      if [ -z "''${CLOUDFLARE_API_TOKEN:-}" ]; then
        if [ -n "''${CLOUDFLARE_API_TOKEN_FILE:-}" ] && [ -r "$CLOUDFLARE_API_TOKEN_FILE" ]; then
          export CLOUDFLARE_API_TOKEN="$(${pkgs.coreutils}/bin/cat "$CLOUDFLARE_API_TOKEN_FILE")"
        ${lib.optionalString (cfg.cloudflareToken.secretPath != null) ''
        elif [ -r ${lib.escapeShellArg (toString cfg.cloudflareToken.secretPath)} ]; then
          export CLOUDFLARE_API_TOKEN="$(${pkgs.coreutils}/bin/cat ${lib.escapeShellArg (toString cfg.cloudflareToken.secretPath)})"
      ''}
        ${lib.optionalString (cfg.cloudflareToken.agenixFile != null) ''
        elif [ -r ${lib.escapeShellArg (toString cfg.cloudflareToken.agenixFile)} ]; then
          cloudflare_token="$(decrypt_agenix_file ${lib.escapeShellArg (toString cfg.cloudflareToken.agenixFile)})" || {
            echo "failed to decrypt Cloudflare token agenix source: ${toString cfg.cloudflareToken.agenixFile}" >&2
            echo "set CANIX_DNS_AGE_IDENTITIES to colon-separated age/ssh identity paths to decrypt agenix sources locally" >&2
            exit 1
          }
          export CLOUDFLARE_API_TOKEN="$cloudflare_token"
      ''}
        else
          echo "CLOUDFLARE_API_TOKEN is unset and no readable Cloudflare token file is available" >&2
          echo "set CANIX_DNS_AGE_IDENTITIES to colon-separated age/ssh identity paths to decrypt agenix sources locally" >&2
          exit 1
        fi
      fi

      exec ${pkgs.octodns.withProviders (_: [pkgs.octodns-providers.cloudflare])}/bin/octodns-sync --config-file "$workdir/config/config.yaml" "$@"
    '';

  octodnsSync = mkOctodnsSync {};

  mkOctodnsConfig = dnsGenerate.cloudflare {
    dnsConfig = validatedDnsConfig;
    inherit token;
    zones = cloudflareZones;
  };

  octodnsConfig = mkOctodnsConfig;

  octodnsConfigLocal = pkgs.linkFarm "cloudflare-octodns-local" [
    {
      name = "config.yaml";
      path = "${mkOctodnsConfig}/config.yaml";
    }
    {
      name = "zones";
      path = "${mkOctodnsConfig}/zones";
    }
    {
      name = "octodns-sync-cloudflare";
      path = mkOctodnsSync {cache = cfg.localCache;};
    }
  ];

  reconcilerEnabled = cfg.enable && cfg.cloudflareToken.secretPath != null;
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
      Environment = "CLOUDFLARE_API_TOKEN_FILE=${cfg.cloudflareToken.secretPath}";
      ExecStart = "${octodnsSync} ${cfg.octodnsConfig} ${extraArgs}";
    };
  };
in {
  options.canix-toolbelt.dns = {
    enable = mkEnableOption "declarative DNS zones via dns-manager";

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

    autoSynthesizeCodebergPagesCnames = mkOption {
      type = types.bool;
      default = true;
      description = "Synthesize CNAME records for Codeberg Pages sites from the codebergPagesSites registry.";
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
            description = "Subdomain for the Codeberg Pages site (e.g. 'myproject' for myproject.tartanoglu.com).";
          };
          targetRepo = mkOption {
            type = types.str;
            description = "Codeberg repository (e.g. 'caniko/myproject').";
          };
        };
      });
      default = [];
      description = "Registry of Codeberg Pages sites. Each entry generates a CNAME from <subdomain>.<zone> to <repoName>.caniko.codeberg.page.";
    };

    reconciler = {
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
      description = "Generated octoDNS Cloudflare configuration directory.";
    };

    octodnsConfigLocal = mkOption {
      type = types.package;
      readOnly = true;
      description = "Generated octoDNS Cloudflare configuration directory with the local planning wrapper.";
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
      dnsConfig = validatedDnsConfig;
      cloudflareZoneExcludes = mapAttrs (_: zone: zone.excludeRecords) cloudflareZones;
      inherit octodnsConfig;
      inherit octodnsConfigLocal;
      agenix.records = agenixSecretRecords;
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
          extraArgs = applyExtraArgs;
        })
        // {
          wantedBy = ["multi-user.target"];
          restartTriggers = [cfg.octodnsConfig];
          unitConfig.ConditionPathExists = toString cfg.cloudflareToken.secretPath;
        };
    };
  };
}
