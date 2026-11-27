{pkgs}: let
  inherit (pkgs) lib;
  evaluate = settings:
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      inherit pkgs;
      system = null;
      modules = [
        ../modules/nixos/services/attic-cache-client.nix
        {canix-toolbelt.services.atticCacheClient = settings;}
      ];
    }).config;
  public = evaluate {
    enable = true;
    substituters = ["https://cache.example/public"];
    trustedPublicKeys = ["public:test-key"];
  };
  privateSettings = {
    enable = true;
    privateCaches.project = {
      url = "https://cache.example/project";
      credentialUrls = ["http://cache.example:8080/project" "https://alias.example/project"];
      publicKey = "project:test-key";
      tokenFile = "/run/secrets/project-token";
    };
  };
  private = evaluate privateSettings;
  duplicate = evaluate (lib.recursiveUpdate privateSettings {
    privateCaches.other = {
      url = "https://CACHE.example/other";
      tokenFile = "/run/secrets/other-token";
    };
  });
  hook = pkgs.writeShellScript "attic-push-hook" "exit 0";
  push = evaluate (privateSettings
    // {
      pushHook = {
        enable = true;
        program = toString hook;
      };
    });
  script = private.system.activationScripts.atticCacheClientNetrc.text;
in
  assert lib.elem "https://cache.example/public" public.nix.settings.substituters;
  assert lib.elem "public:test-key" public.nix.settings.trusted-public-keys;
  assert !(public.nix.settings ? netrc-file);
  assert !(public.system.activationScripts ? atticCacheClientNetrc);
  assert private.nix.settings.netrc-file == "/run/nix/attic-netrc";
  assert lib.elem "https://cache.example/project" private.nix.settings.substituters;
  assert !lib.elem "https://alias.example/project" private.nix.settings.substituters;
  assert lib.elem "project:test-key" private.nix.settings.trusted-public-keys;
  assert lib.all (needle: lib.hasInfix needle script) [
    "/run/secrets/project-token"
    "machine cache.example"
    "machine alias.example"
    "chmod 0600"
    "mv -fT"
    "cmp -s"
    "try-restart nix-daemon.service"
  ];
  assert lib.any (entry: !entry.assertion && lib.hasInfix "conflicting token files for cache.example" entry.message) duplicate.assertions;
  assert !(public.nix.settings ? post-build-hook);
  assert !(private.nix.settings ? post-build-hook);
  assert push.nix.settings.post-build-hook == toString hook;
    pkgs.runCommand "attic-cache-client-eval" {} ''touch "$out"''
