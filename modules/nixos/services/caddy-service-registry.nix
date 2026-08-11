{
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkIf mkMerge mkOption nameValuePair types;
  caddyLib = import ../../../lib/caddy.nix {inherit lib;};
  cfg = config.canix-toolbelt.services.caddy;
  serviceCfg = config.canix-toolbelt.services;
  oidcCfg = cfg.oidc;

  dialAddress = svc: let
    isLocal =
      if svc.local != null
      then svc.local
      else svc.targetHost == config.networking.hostName;
  in
    if isLocal
    then "127.0.0.1"
    else config.canix-toolbelt.hosts.${svc.targetHost}.lanIp;

  reverseProxyRoute = svc:
    if svc.auth.enable
    then
      caddyLib.mkAuthServiceRoute {
        inherit (svc) hostname upstreamScheme tlsServerName;
        port =
          if svc.proxied
          then 80
          else svc.port;
        host = dialAddress svc;
        portalName = svc.name;
      }
    else
      caddyLib.mkReverseProxyRoute {
        inherit (svc) hostname upstreamScheme tlsServerName;
        port =
          if svc.proxied
          then 80
          else svc.port;
        host = dialAddress svc;
        inherit (cfg) goatcounterUrl;
        injectAnalytics = cfg.goatcounterUrl != null;
      };

  staticFileRoute = svc:
    caddyLib.mkStaticFileRoute {
      inherit (svc) hostname;
      root = svc.staticRoot;
    };

  caddyRoutes =
    (map reverseProxyRoute serviceCfg.reverseProxyServices)
    ++ (map staticFileRoute serviceCfg.staticFileServices);

  nonCloudflareHostnames = lib.unique (
    map (svc: svc.hostname) (
      lib.filter (svc: !svc.cloudflareProxied) (
        serviceCfg.reverseProxyServices ++ serviceCfg.staticFileServices
      )
    )
  );

  hasAuthServices = lib.any (svc: svc.auth.enable) serviceCfg.reverseProxyServices;
  kanidmAuthServices = lib.filter (svc: svc.auth.enable && svc.auth.provider == "kanidm") serviceCfg.reverseProxyServices;
  unsupportedAuthServices = lib.filter (svc: svc.auth.enable && svc.auth.provider != "kanidm") serviceCfg.reverseProxyServices;

  caddySecurityPlugin = "github.com/greenpau/caddy-security@v1.1.62";

  secretAttrName = name: "${name}-oidc-client-secret";
  secretPlaintextName = name: builtins.replaceStrings ["-"] ["_"] (secretAttrName name);
  envVarName = name: lib.toUpper (builtins.replaceStrings ["-" "."] ["_" "_"] (secretAttrName name));

  oidcClientFor = svc: {
    clientSecretFile = config.age.secrets.${secretAttrName svc.name}.path;
    envVar = envVarName svc.name;
  };

  oidcClients = builtins.listToAttrs (
    map (svc: {
      inherit (svc) name;
      value = oidcClientFor svc;
    })
    kanidmAuthServices
  );

  oidcEnvFile = "/run/canix-caddy-oidc/env";
  oidcEnvService = "canix-caddy-oidc-env";
  renderOidcEnv = let
    renderOne = svc: ''
      val="$(tr -d '\n' < ${lib.escapeShellArg oidcClients.${svc.name}.clientSecretFile})"
      printf '%s=%s\n' ${lib.escapeShellArg oidcClients.${svc.name}.envVar} "$val" >> "$tmp"
    '';
  in ''
    set -eu
    umask 077
    tmp="${oidcEnvFile}.tmp"
    : > "$tmp"
    ${lib.concatMapStrings renderOne kanidmAuthServices}
    chmod 0400 "$tmp"
    mv "$tmp" ${lib.escapeShellArg oidcEnvFile}
  '';
in {
  imports = [
    ./caddy-base.nix
  ];

  options.canix-toolbelt.services.caddy = {
    useServiceRegistry = mkOption {
      type = types.bool;
      default = false;
      description = "Whether to synthesize Caddy routes from the canix-toolbelt service registry.";
    };

    oidc = {
      enable = mkOption {
        type = types.bool;
        default = cfg.useServiceRegistry;
        defaultText = lib.literalExpression "config.canix-toolbelt.services.caddy.useServiceRegistry";
        description = "Whether to synthesize OIDC clients for authenticated service-registry routes.";
      };

      kanidmDomain = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Kanidm domain used for synthesized Caddy OIDC provider metadata URLs.";
      };

      secretPath = mkOption {
        type = types.functionTo types.path;
        default = name: throw "canix-toolbelt.services.caddy.oidc.secretPath is required for authenticated OIDC service ${name}";
        description = "Function mapping a service name to its agenix source file.";
      };

      clients = mkOption {
        type = types.attrsOf (types.submodule {
          options = {
            clientSecretFile = mkOption {
              type = types.str;
              description = "Runtime path to the generated OIDC client secret.";
            };

            envVar = mkOption {
              type = types.str;
              description = "Environment variable used by caddy-security for this client secret.";
            };
          };
        });
        default = {};
        description = "Generated Caddy OIDC client metadata keyed by service name.";
      };

      environmentFile = mkOption {
        type = types.str;
        default = oidcEnvFile;
        readOnly = true;
        description = "Runtime Caddy EnvironmentFile containing generated OIDC client secrets.";
      };
    };
  };

  config = mkIf cfg.useServiceRegistry (mkMerge [
    {
      assertions = [
        {
          assertion = !oidcCfg.enable || kanidmAuthServices == [] || oidcCfg.kanidmDomain != null;
          message = "canix-toolbelt.services.caddy.oidc.kanidmDomain is required when authenticated Kanidm service-registry routes are enabled.";
        }
        {
          assertion = !oidcCfg.enable || unsupportedAuthServices == [];
          message = "canix-toolbelt.services.caddy.oidc only supports auth.provider = \"kanidm\"; unsupported authenticated services: ${lib.concatMapStringsSep ", " (svc: "${svc.name} (${svc.auth.provider})") unsupportedAuthServices}";
        }
      ];

      canix-toolbelt.services.caddy = {
        routes = caddyRoutes;
        cidrExemptHosts = nonCloudflareHostnames;

        # Auto-add caddy-security plugin when any service uses auth.
        plugins = mkIf hasAuthServices (lib.mkBefore [caddySecurityPlugin]);
      };
    }
    (mkIf (oidcCfg.enable && kanidmAuthServices != []) {
      age.secrets = builtins.listToAttrs (
        map (svc:
          nameValuePair (secretAttrName svc.name) {
            name = secretPlaintextName svc.name;
            rekeyFile = oidcCfg.secretPath svc.name;
            generator.script = "alnum";
            owner = "root";
            group = "kanidm";
            mode = "0440";
          })
        kanidmAuthServices
      );

      canix-toolbelt.services.caddy = {
        oidc.clients = oidcClients;
        authProviders = builtins.listToAttrs (
          map (svc:
            nameValuePair svc.name {
              driver = "generic";
              client_id = svc.name;
              metadata_url = "https://${oidcCfg.kanidmDomain}/oauth2/openid/${svc.name}/.well-known/openid-configuration";
              scopes = [
                "openid"
                "profile"
                "email"
              ];
              client_secret = "{env.${oidcClients.${svc.name}.envVar}}";
            })
          kanidmAuthServices
        );
      };

      systemd.services.${oidcEnvService} = {
        description = "Render Caddy OIDC client secret environment";
        before = ["caddy.service"];
        wantedBy = ["caddy.service"];
        path = [pkgs.coreutils];

        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
          RuntimeDirectory = "canix-caddy-oidc";
          RuntimeDirectoryMode = "0700";
        };

        script = renderOidcEnv;
      };

      systemd.services.caddy = {
        requires = ["${oidcEnvService}.service"];
        after = ["${oidcEnvService}.service"];
        serviceConfig.EnvironmentFile = oidcCfg.environmentFile;
      };
    })
  ]);
}
