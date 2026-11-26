{
  inputs,
  pkgs,
}: let
  inherit (pkgs) lib;
  providers = inputs.self.lib.dnsProviders;
  mkSystem = dnsManager: dns:
    (inputs.nixpkgs.lib.nixosSystem {
      inherit (pkgs.stdenv.hostPlatform) system;
      specialArgs = {
        inputs = inputs // {dns-manager.lib.generate = _: dnsManager;};
        crossbowBuildPkgs = pkgs;
        canixCrossPackage = _: package: package;
      };
      modules = [
        inputs.self.nixosModules.dns-ddns
        inputs.self.nixosModules.dns-octodns-cloudflare
        {
          system.stateVersion = "25.11";
          canix-toolbelt.dns = {enable = true;} // dns;
        }
      ];
    }).config;
  legacy = {
    cloudflare = args: pkgs.writeText "legacy-cloudflare" (builtins.toJSON (removeAttrs args ["dnsConfig"]));
  };
  old =
    (mkSystem legacy {
      cloudflareToken.secretPath = "/run/secrets/legacy";
      zones."example.org" = {};
    }).canix-toolbelt.dns;
  modern =
    legacy
    // {
      sync = args:
        assert builtins.attrNames args.providers == ["cloudflare" "eu" "hetzner"];
        assert args.extraGlobalConfig.processors.drop-ignored.class == "canix_octodns_drop_ignored.DropIgnoredRecordsProcessor";
        assert args.zones."example.org"
        == {
          targets = ["cloudflare" "eu"];
          mode = "lenient";
          manageRecordTypes = null;
          processors = ["drop-ignored"];
          excludeRecords = [
            {
              name = "home";
              type = "A";
            }
          ];
        };
        assert args.zones."eu.example.org".targets == ["eu"];
        assert args.providers.eu
        == {
          kind = "desec";
          token = {
            type = "file";
            path = "/run/secrets/eu";
            envName = "EU_DNS_TOKEN";
          };
          settings = {};
          minTtlOverride = 3600;
        };
          pkgs.writeText "multi-provider-sync" (builtins.toJSON (removeAttrs args ["dnsConfig"]));
    };
  settings = {
    providers = {
      cloudflare.token.secretPath = "/run/secrets/cf";
      eu = {
        kind = "desec";
        token.secretPath = "/run/secrets/eu";
        token.envName = "EU_DNS_TOKEN";
        acmeTokenPath = "/run/secrets/eu-acme";
        ddnsTokenPath = "/run/secrets/eu-ddns";
        minTtlOverride = 3600;
      };
      hetzner = {
        kind = "hetzner";
        token.secretPath = "/run/secrets/hetzner";
      };
      unused.kind = "bunny";
    };
    zones = {
      "example.org" = {
        migrateTo = "eu";
        exclude = [
          {
            name = "home";
            type = "A";
          }
        ];
      };
      "eu.example.org" = {
        provider = "eu";
        exclude = [
          {
            name = "home";
            type = "A";
          }
          {
            name = "home";
            type = "AAAA";
          }
        ];
      };
      "example.net" = {
        provider = "hetzner";
        exclude = [
          {
            name = "@";
            type = "AAAA";
          }
        ];
      };
    };
    ddns = {
      enable = true;
      hosts = {
        home = {
          fqdn = "home.eu.example.org";
          provider = "eu";
          families = ["ipv4"];
        };
        edge = {
          fqdn = "example.net";
          provider = "hetzner";
          families = ["ipv6"];
        };
        cf = {
          fqdn = "home.example.org";
          provider = "cloudflare";
          families = ["ipv4"];
          proxied = true;
        };
      };
    };
  };
  mixed = mkSystem modern settings;
  ddnsOnly = mkSystem legacy (settings // {enable = false;});
  dns = mixed.canix-toolbelt.dns;
  oldMixed = (mkSystem legacy settings).canix-toolbelt.dns;
  service = name: mixed.systemd.services.${name}.serviceConfig;
  cf = mixed.services.cloudflare-ddns;
  missingCredential = mkSystem modern {
    providers.gcore = {};
    zones."example.org".provider = "gcore";
    ddns = {
      enable = true;
      hosts.home = {
        fqdn = "home.example.org";
        provider = "gcore";
      };
    };
  };
in
  assert builtins.attrNames providers == ["bunny" "cloudflare" "desec" "gcore" "hetzner"];
  assert builtins.all (kind: providers.${kind}.octodnsProviderAttr == kind && providers.${kind}.legoProvider == kind && providers.${kind}.ddnsKind == kind) (builtins.attrNames providers);
  assert builtins.filter (kind: providers.${kind}.supportsProxy) (builtins.attrNames providers) == ["cloudflare"];
  assert providers.gcore.legoEnvFileVar == "GCORE_PERMANENT_API_TOKEN_FILE";
  assert providers.hetzner.octodnsDefaultEnv == "HETZNER_API_TOKEN";
  assert providers.bunny.stalwartType == "BunnyDNS";
  assert old.providers.cloudflare.token.secretPath == "/run/secrets/legacy";
  assert old.cloudflareToken == old.providers.cloudflare.token;
  assert old.octodnsConfig.name == "legacy-cloudflare";
  assert !(builtins.tryEval oldMixed.octodnsConfig).success;
  assert dns.octodnsConfig.name == "multi-provider-sync";
  assert lib.hasSuffix ".drv" dns.octodnsConfigLocal.drvPath;
  assert dns.zones."example.org".provider == "cloudflare";
  assert dns.providers.eu.token.envName == "EU_DNS_TOKEN";
  assert dns.providers.hetzner.token.envName == "HETZNER_API_TOKEN";
  assert dns.acmeFor "*.EU.Example.Org."
  == {
    dnsProvider = "desec";
    credentialFiles.DESEC_TOKEN_FILE = "/run/secrets/eu-acme";
  };
  assert dns.acmeFor "example.org"
  == {
    dnsProvider = "cloudflare";
    credentialFiles.CLOUDFLARE_DNS_API_TOKEN_FILE = "/run/secrets/cf";
  };
  assert !(builtins.tryEval (dns.acmeFor "notexample.org")).success;
  assert dns.stalwartDnsFor "eu.example.org"
  == {
    type = "DeSEC";
    tokenPath = "/run/secrets/eu-acme";
  };
  assert (service "dns-ddns-home").LoadCredential == ["DESEC_TOKEN:/run/secrets/eu-ddns"];
  assert (service "dns-ddns-edge").LoadCredential == ["HETZNER_API_TOKEN:/run/secrets/hetzner"];
  assert (service "cloudflare-ddns").LoadCredential == ["CLOUDFLARE_API_TOKEN:/run/secrets/cf"];
  assert builtins.elem "CLOUDFLARE_API_TOKEN_FILE=%d/CLOUDFLARE_API_TOKEN" (service "cloudflare-ddns").Environment;
  assert (service "cloudflare-octodns").LoadCredential == ["CLOUDFLARE_API_TOKEN:/run/secrets/cf" "EU_DNS_TOKEN:/run/secrets/eu" "HETZNER_API_TOKEN:/run/secrets/hetzner"];
  assert mixed.systemd.timers.dns-ddns-home.timerConfig.OnUnitInactiveSec == "5min";
  assert builtins.all (entry: entry.assertion || !(lib.hasPrefix "canix-toolbelt.dns" entry.message)) mixed.assertions;
  assert builtins.any (entry: !entry.assertion && lib.hasInfix "providers.gcore.ddnsTokenPath" entry.message) missingCredential.assertions;
  assert cf.ip4Domains == ["home.example.org"] && cf.ip6Domains == [] && cf.provider.ipv6 == "none";
  assert cf.proxied == "is(home.example.org)";
  assert ddnsOnly.systemd.services ? dns-ddns-home && !(ddnsOnly.systemd.services ? cloudflare-octodns);
    pkgs.runCommand "dns-multi-provider-eval" {
      nativeBuildInputs = [
        (pkgs.python3.withPackages (_: [
          pkgs.octodns
          pkgs.octodns-providers.cloudflare
        ]))
      ];
    } ''
      python3 - ${../modules/nixos/services/dns-octodns-drop-ignored.py} ${../modules/nixos/services/dns-octodns-cloudflare-substitute-secrets.py} <<'PY'
      import importlib.util
      import json
      import pathlib
      import subprocess
      import sys
      import tempfile

      from octodns.record import Record
      from octodns.zone import Zone
      from octodns_cloudflare import CloudflareProvider

      drop_path, sub_path = sys.argv[1], sys.argv[2]
      spec = importlib.util.spec_from_file_location("canix_octodns_drop_ignored", drop_path)
      mod = importlib.util.module_from_spec(spec)
      spec.loader.exec_module(mod)
      proc = mod.DropIgnoredRecordsProcessor("drop-ignored")

      provider = CloudflareProvider("cloudflare", token="dummy")
      desired = Zone("example.org.", [])
      existing = Zone("example.org.", [])

      # Ignored DDNS placeholder A record vs live Cloudflare proxied/auto-ttl A record,
      # plus a managed TXT record at the same owner name (@).
      desired.add_record(Record.new(desired, "", {
          "type": "A",
          "ttl": 1,
          "value": "192.0.2.1",
          "octodns": {"ignored": True},
      }))
      desired.add_record(Record.new(desired, "", {
          "type": "TXT",
          "ttl": 300,
          "value": "v=spf1 -all",
      }))
      existing.add_record(Record.new(existing, "", {
          "type": "A",
          "ttl": 300,
          "value": "89.245.208.74",
          "octodns": {"cloudflare": {"auto-ttl": True, "proxied": True}},
      }))
      existing.add_record(Record.new(existing, "", {
          "type": "TXT",
          "ttl": 300,
          "value": "v=spf1 -all",
      }))

      # Without DropIgnoredRecordsProcessor, upstream octodns-cloudflare._extra_changes
      # emits a bogus Update overwriting the live A record with 192.0.2.1.
      raw_changes = existing.changes(desired, provider)
      assert raw_changes == []
      buggy_extra = provider._extra_changes(existing, desired, raw_changes)
      assert len(buggy_extra) == 1 and buggy_extra[0].new.values == ["192.0.2.1"]

      # With DropIgnoredRecordsProcessor, the ignored A record is stripped from both
      # zones while the managed TXT record at the same owner remains managed.
      d2, e2 = proc.process_source_and_target_zones(desired.copy(), existing.copy(), provider)
      assert {(r.name, r._type) for r in d2.records} == {("", "TXT")}
      assert {(r.name, r._type) for r in e2.records} == {("", "TXT")}
      c2 = e2.changes(d2, provider)
      assert c2 == []
      assert provider._extra_changes(e2, d2, c2) == []

      # Verify secret substitution when no zone filter is passed (with and without --target).
      with tempfile.TemporaryDirectory() as tmp:
          root = pathlib.Path(tmp)
          secret = root / "brevo.txt"
          secret.write_text("brevo-code:test-value\n")
          repl = root / "replacements.json"
          repl.write_text(json.dumps([{
              "placeholder": "__SECRET_BREVO__",
              "path": str(secret),
              "agenixFile": None,
          }]))
          for args, expect_sub in [
              (["--doit"], True),
              (["--doit", "--target", "cloudflare"], True),
              (["--doit", "--target", "cloudflare", "other.org."], False),
              (["--doit", "--target", "cloudflare", "example.org."], True),
          ]:
              cfg = root / f"cfg-{len(args)}-{expect_sub}"
              zones = cfg / "zones"
              zones.mkdir(parents=True, exist_ok=True)
              (zones / "example.org.yaml").write_text('"":\n  - type: TXT\n    value: "__SECRET_BREVO__"\n')
              (zones / "other.org.yaml").write_text('"":\n  - type: TXT\n    value: "plain"\n')
              subprocess.run(
                  [sys.executable, sub_path, str(cfg), str(zones), str(zones), "-", str(repl), *args],
                  check=True,
              )
              rendered = (zones / "example.org.yaml").read_text()
              if expect_sub:
                  assert "brevo-code:test-value" in rendered and "__SECRET_BREVO__" not in rendered, (args, rendered)
              else:
                  assert "__SECRET_BREVO__" in rendered, (args, rendered)
      PY
      echo ok > "$out"
    ''
