let
  provider = kind: octodnsDefaultEnv: legoEnvFileVar: stalwartType: {
    inherit octodnsDefaultEnv legoEnvFileVar stalwartType;
    octodnsProviderAttr = kind;
    legoProvider = kind;
    supportsProxy = kind == "cloudflare";
    ddnsKind = kind;
  };
in {
  cloudflare = provider "cloudflare" "CLOUDFLARE_API_TOKEN" "CLOUDFLARE_DNS_API_TOKEN_FILE" "Cloudflare";
  desec = provider "desec" "DESEC_TOKEN" "DESEC_TOKEN_FILE" "DeSEC";
  hetzner = provider "hetzner" "HETZNER_API_TOKEN" "HETZNER_API_TOKEN_FILE" "Hetzner";
  gcore = provider "gcore" "GCORE_API_KEY" "GCORE_PERMANENT_API_TOKEN_FILE" "Gcore";
  bunny = provider "bunny" "BUNNY_API_KEY" "BUNNY_API_KEY_FILE" "BunnyDNS";
}
