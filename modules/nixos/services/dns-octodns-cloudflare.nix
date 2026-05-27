{
  config,
  inputs,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.dns;

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
    hashString
    literalExpression
    mapAttrs
    mkEnableOption
    mkOption
    removeSuffix
    splitString
    types
    ;

  dnsGenerate = inputs.nixos-dns.utils.generate pkgs;

  recordTypes = [
    "A"
    "AAAA"
    "ALIAS"
    "CAA"
    "CNAME"
    "DNAME"
    "MX"
    "NS"
    "SOA"
    "SRV"
    "SSHFP"
    "TLSA"
    "TXT"
    "URI"
  ];

  proxiableRecordTypes = [
    "A"
    "AAAA"
    "ALIAS"
    "CNAME"
  ];

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

      comment = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional provider or generated-zone comment.";
      };

      preference = mkOption {
        type = types.nullOr types.int;
        default = null;
        description = "MX record preference.";
      };

      exchange = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "MX record exchange host. Defaults to data when omitted.";
      };

      priority = mkOption {
        type = types.nullOr types.int;
        default = null;
        description = "SRV or URI record priority.";
      };

      weight = mkOption {
        type = types.nullOr types.int;
        default = null;
        description = "SRV or URI record weight.";
      };

      port = mkOption {
        type = types.nullOr types.int;
        default = null;
        description = "SRV record port.";
      };

      target = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "SRV or URI target. Defaults to data when omitted.";
      };

      flags = mkOption {
        type = types.nullOr types.int;
        default = null;
        description = "CAA record flags.";
      };

      tag = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "CAA record tag.";
      };

      value = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "CAA record value. Defaults to data when omitted.";
      };

      usage = mkOption {
        type = types.nullOr types.int;
        default = null;
        description = "TLSA certificate usage.";
      };

      selector = mkOption {
        type = types.nullOr types.int;
        default = null;
        description = "TLSA selector.";
      };

      matchingType = mkOption {
        type = types.nullOr types.int;
        default = null;
        description = "TLSA matching type.";
      };

      certificateAssociationData = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "TLSA certificate association data.";
      };

      algorithm = mkOption {
        type = types.nullOr types.int;
        default = null;
        description = "SSHFP algorithm.";
      };

      fingerprintType = mkOption {
        type = types.nullOr types.int;
        default = null;
        description = "SSHFP fingerprint type.";
      };

      fingerprint = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "SSHFP fingerprint.";
      };
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
      ttl =
        if record.ttlAuto
        then null
        else if record.ttl != null
        then record.ttl
        else zone.defaultTtl;
      ttlAuto =
        if record.ttlAuto
        then true
        else null;
      proxied =
        if elem (normalizeType record.type) proxiableRecordTypes
        then record.proxied
        else null;
      inherit (record) comment;
    };

  secretPlaceholderForRecord = record: "__CANIX_DNS_SECRET_${hashString "sha256" "${normalizeName record.name}|${normalizeType record.type}|${toString record.dataFile}|${toString record.dataAgenixFile}"}__";

  valueForRecord = record:
    if record.dataFile != null || record.dataAgenixFile != null
    then secretPlaceholderForRecord record
    else record.data;

  dataForRecord = record: let
    type = normalizeType record.type;
    data = valueForRecord record;
  in
    if type == "MX"
    then {
      inherit (record) preference;
      exchange = valueOr data record.exchange;
    }
    else if type == "SRV"
    then {
      inherit (record) priority;
      inherit (record) weight;
      inherit (record) port;
      target = valueOr data record.target;
    }
    else if type == "URI"
    then {
      inherit (record) priority;
      inherit (record) weight;
      target = valueOr data record.target;
    }
    else if type == "CAA"
    then {
      inherit (record) flags;
      inherit (record) tag;
      value = valueOr data record.value;
    }
    else if type == "TLSA" && builtins.isAttrs data
    then data
    else if type == "TLSA"
    then {
      inherit (record) usage;
      inherit (record) selector;
      inherit (record) matchingType;
      certificateAssociationData = valueOr data record.certificateAssociationData;
    }
    else if type == "SSHFP" && builtins.isAttrs data
    then data
    else if type == "SSHFP"
    then {
      inherit (record) algorithm;
      type = record.fingerprintType;
      fingerprint = valueOr data record.fingerprint;
    }
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

  secretReplacements =
    builtins.map (record: {
      placeholder = secretPlaceholderForRecord record;
      path =
        if record.dataFile == null
        then null
        else toString record.dataFile;
      agenixFile =
        if record.dataAgenixFile == null
        then null
        else toString record.dataAgenixFile;
    })
    secretRecordFiles;

  secretReplacementsJson = pkgs.writeText "cloudflare-octodns-secret-records.json" (builtins.toJSON secretReplacements);

  substituteDnsSecrets = pkgs.writeText "cloudflare-octodns-substitute-secrets.py" ''
    import json
    import os
    import pathlib
    import subprocess
    import sys

    config_dir = pathlib.Path(sys.argv[1])
    source_zones_dir = pathlib.Path(sys.argv[2])
    runtime_zones_dir = pathlib.Path(sys.argv[3])
    requested_zone_files = {
        f"{zone[:-1] if zone.endswith('.') else zone}.yaml"
        for zone in sys.argv[4:]
        if zone and not zone.startswith("-")
    }
    source_zone_paths = {
        str(source_zones_dir),
        str(source_zones_dir.resolve()),
    }
    replacements = json.loads(pathlib.Path("${secretReplacementsJson}").read_text())
    secret_dir = os.environ.get("CANIX_DNS_SECRET_DIR")
    age_identities = [
        identity
        for identity in os.environ.get("CANIX_DNS_AGE_IDENTITIES", "").split(":")
        if identity
    ]

    def read_secret(replacement):
        runtime_path = replacement.get("path")
        if runtime_path is not None:
            path = pathlib.Path(runtime_path)
            if path.exists():
                return path.read_text().strip()
            if secret_dir is not None:
                path = pathlib.Path(secret_dir) / pathlib.Path(runtime_path).name
                if path.exists():
                    return path.read_text().strip()

        agenix_file = replacement.get("agenixFile")
        if agenix_file is not None:
            path = pathlib.Path(agenix_file)
            if not path.exists():
                print(f"missing DNS agenix source file: {path}", file=sys.stderr)
                sys.exit(1)
            readable_identities = [
                identity
                for identity in age_identities
                if pathlib.Path(identity).is_file() and os.access(identity, os.R_OK)
            ]
            if readable_identities:
                cmd = ["${pkgs.rage}/bin/rage", "--decrypt"]
                for identity in readable_identities:
                    cmd.extend(["--identity", identity])
                cmd.append(str(path))
                result = subprocess.run(cmd, check=False, text=True, capture_output=True)
                if result.returncode == 0:
                    return result.stdout.strip()
                print(result.stderr, file=sys.stderr, end="")
                print(f"failed to decrypt DNS agenix source file: {path}", file=sys.stderr)
                sys.exit(result.returncode)

        missing = runtime_path or agenix_file
        print(f"missing DNS secret file: {missing}", file=sys.stderr)
        if agenix_file is not None and not age_identities:
            print("set CANIX_DNS_AGE_IDENTITIES to colon-separated age/ssh identity paths to decrypt the agenix source locally", file=sys.stderr)
        sys.exit(1)

    for path in config_dir.rglob("*.yaml"):
        text = path.read_text()
        for source_zone_path in source_zone_paths:
            text = text.replace(source_zone_path, str(runtime_zones_dir))
        should_substitute_secrets = (
            not requested_zone_files
            or path.parent.name != "zones"
            or path.name in requested_zone_files
        )
        if not should_substitute_secrets:
            path.write_text(text)
            continue
        for replacement in replacements:
            if replacement["placeholder"] not in text:
                continue
            value = json.dumps(read_secret(replacement))[1:-1]
            text = text.replace(replacement["placeholder"], value)
        path.write_text(text)
  '';

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

  rawOctodnsConfig = dnsGenerate.cloudflareConfig {
    dnsConfig = validatedDnsConfig;
    inherit token;
    zones = cloudflareZones;
  };

  defaultAgeIdentities = concatStringsSep ":" (builtins.map toString ((config.age.identityPaths or []) ++ ["$HOME/.ssh/id_ssh_ed25519"]));

  octodnsPkg = pkgs.octodns.withProviders (_: [pkgs.octodns-providers.cloudflare]);
  octodnsSync = pkgs.writeShellScript "cloudflare-octodns-sync" ''
    set -eu
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
    export CANIX_DNS_AGE_IDENTITIES="''${CANIX_DNS_AGE_IDENTITIES:-${defaultAgeIdentities}}"
    ${pkgs.python3}/bin/python ${substituteDnsSecrets} "$workdir/config" "$source_config/zones" "$workdir/config/zones" "$@"

    decrypt_agenix_file() {
      encrypted_file="$1"
      if [ -z "$CANIX_DNS_AGE_IDENTITIES" ]; then
        return 1
      fi
      old_ifs="$IFS"
      IFS=:
      for identity in $CANIX_DNS_AGE_IDENTITIES; do
        IFS="$old_ifs"
        if [ -r "$identity" ] && ${pkgs.rage}/bin/rage --decrypt --identity "$identity" "$encrypted_file"; then
          return 0
        fi
        IFS=:
      done
      IFS="$old_ifs"
      return 1
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

    exec ${octodnsPkg}/bin/octodns-sync --config-file "$workdir/config/config.yaml" "$@"
  '';

  octodnsConfig = pkgs.runCommand "cloudflare-octodns" {} ''
    ${pkgs.coreutils}/bin/mkdir -p "$out"
    ${pkgs.coreutils}/bin/cp -R --no-preserve=mode ${rawOctodnsConfig}/. "$out"
    ${pkgs.coreutils}/bin/chmod -R u+w "$out"
    ${pkgs.coreutils}/bin/rm -f "$out/octodns-sync-cloudflare"
    ${pkgs.coreutils}/bin/install -m 0755 ${octodnsSync} "$out/octodns-sync-cloudflare"
  '';

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
