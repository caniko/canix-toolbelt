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
        assert args.zones."example.org"
        == {
          targets = ["cloudflare" "eu"];
          mode = "lenient";
          manageRecordTypes = null;
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
    pkgs.writeText "dns-multi-provider-eval" "ok"
