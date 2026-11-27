{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkOption types escapeShellArg;
  cfg = config.canix-toolbelt.services.atticCacheClient;
  caches = builtins.attrValues cfg.privateCaches;
  hostFromUrl = url: let
    parts = builtins.match "https?://([[][0-9a-fA-F:]+[]]|[a-zA-Z0-9._-]+)(:[0-9]+)?(/[^[:space:]]*)?" url;
  in
    if parts == null
    then throw "canix-toolbelt.services.atticCacheClient: expected a credential URL with an HTTP(S) hostname and no userinfo."
    else lib.toLower (lib.removeSuffix "]" (lib.removePrefix "[" (builtins.head parts)));

  credentials = lib.groupBy (entry: entry.host) (lib.concatLists (lib.mapAttrsToList (name: cache:
    map (url: {
      host = hostFromUrl url;
      inherit name;
      inherit (cache) tokenFile;
    }) (lib.unique ([cache.url] ++ cache.credentialUrls)))
  cfg.privateCaches));

  netrc = escapeShellArg cfg.netrcFile;
  coreutils = "${pkgs.coreutils}/bin";
in {
  options.canix-toolbelt.services.atticCacheClient = {
    enable = lib.mkEnableOption "Attic cache consumption";
    substituters = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "Public cache URLs to add as Nix substituters.";
    };
    trustedPublicKeys = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "Trusted cache signing keys.";
    };
    netrcFile = mkOption {
      type = types.strMatching "/[^[:space:]]+";
      default = "/run/nix/attic-netrc";
      description = "Absolute path for the generated private cache netrc. Its parent and ancestors must not be writable by unprivileged users.";
    };
    privateCaches = mkOption {
      default = {};
      description = "Private caches and their runtime credential files, keyed by cache name.";
      type = types.attrsOf (types.submodule ({config, ...}: {
        options = {
          url = mkOption {
            type = types.str;
            description = "Private cache URL to add as a substituter.";
          };
          credentialUrls = mkOption {
            type = types.listOf types.str;
            default = [config.url];
            defaultText = lib.literalExpression "[ url ]";
            description = "URLs that use the same token without adding substituters. The cache URL always uses this token.";
          };
          publicKey = mkOption {
            type = types.nullOr types.str;
            default = null;
            description = "Trusted signing key for this cache, if not configured separately.";
          };
          tokenFile = mkOption {
            type = types.strMatching "/run/[^[:space:]]+";
            description = "Path to a runtime-provisioned token. For an activation-time secret provider, order atticCacheClientNetrc after the provider with system.activationScripts.atticCacheClientNetrc.deps.";
          };
        };
      }));
    };
    pushHook = {
      enable = lib.mkEnableOption "setting Nix's global post-build hook to the consumer-supplied executable";
      program = mkOption {
        type = types.nullOr (types.strMatching "/[^[:space:]]+");
        default = null;
        description = "Absolute executable path. The hook must decide which outputs may be published and where.";
      };
    };
  };

  config = lib.mkIf cfg.enable (lib.mkMerge [
    {
      nix.settings = {
        substituters = cfg.substituters ++ map (cache: cache.url) caches;
        trusted-public-keys = cfg.trustedPublicKeys ++ lib.concatMap (cache: lib.optional (cache.publicKey != null) cache.publicKey) caches;
      };
    }
    (lib.mkIf (caches != []) {
      assertions =
        lib.mapAttrsToList (host: entries: {
          assertion = builtins.length (lib.unique (map (entry: entry.tokenFile) entries)) == 1;
          message = "canix-toolbelt.services.atticCacheClient: conflicting token files for ${host}; use one token scoped to the required caches.";
        })
        credentials
        ++ lib.mapAttrsToList (name: cache: {
          assertion =
            builtins.match "[a-zA-Z0-9_-]+" name
            != null
            && !builtins.hasContext cache.tokenFile
            && !lib.elem ".." (lib.splitString "/" cache.tokenFile);
          message = "canix-toolbelt.services.atticCacheClient: use a valid cache name and a runtime token path under /run, not a store dependency.";
        })
        cfg.privateCaches;

      nix.settings.netrc-file = cfg.netrcFile;
      system.activationScripts.atticCacheClientNetrc.text = ''
        (
          set -eu
          umask 077
          ${coreutils}/mkdir -p -- ${escapeShellArg (builtins.dirOf cfg.netrcFile)}
          if [ -L ${netrc} ] || { [ -e ${netrc} ] && [ ! -f ${netrc} ]; }; then
            echo 'Refusing to replace an Attic netrc that is not a regular file.' >&2
            exit 1
          fi
          tmp="$(${coreutils}/mktemp -- ${escapeShellArg "${cfg.netrcFile}.XXXXXX"})"
          trap '${coreutils}/rm -f -- "$tmp"' EXIT
          {
            ${lib.concatStringsSep "\n" (lib.mapAttrsToList (host: entries: let
            entry = builtins.head entries;
          in ''
            token="$(${coreutils}/cat -- ${escapeShellArg entry.tokenFile})"
            case "$token" in
              ""|*[!A-Za-z0-9_.-]*) echo 'Invalid Attic token; keeping existing netrc.' >&2; exit 1 ;;
              *.*.*) ;;
              *) echo 'Malformed Attic token; keeping existing netrc.' >&2; exit 1 ;;
            esac
            printf '%s\n' ${escapeShellArg "machine ${host}"} ${escapeShellArg "login ${entry.name}"}
            printf 'password %s\n' "$token"
          '')
          credentials)}
          } > "$tmp"
          ${coreutils}/chown root:root "$tmp"
          ${coreutils}/chmod 0600 "$tmp"
          if ${pkgs.diffutils}/bin/cmp -s -- "$tmp" ${netrc}; then
            ${coreutils}/chown root:root ${netrc}
            ${coreutils}/chmod 0600 ${netrc}
          else
            ${coreutils}/mv -fT -- "$tmp" ${netrc}
            ${pkgs.systemd}/bin/systemctl try-restart nix-daemon.service
          fi
        )
      '';
    })
    (lib.mkIf cfg.pushHook.enable {
      assertions = [
        {
          assertion = cfg.pushHook.program != null;
          message = "canix-toolbelt.services.atticCacheClient.pushHook.program is required when enabling the global post-build hook.";
        }
      ];
      nix.settings = lib.mkIf (cfg.pushHook.program != null) {
        post-build-hook = cfg.pushHook.program;
      };
    })
  ]);
}
