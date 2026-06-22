{
  config,
  lib,
  ...
}: let
  caddyLib = import ../../../lib/caddy.nix {inherit lib;};
  cfg = config.canix-toolbelt.services.caddy;
  serviceCfg = config.canix-toolbelt.services;

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
        portal = svc.name;
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

  caddySecurityPlugin = "github.com/greenpau/caddy-security@v1.1.62";
in {
  imports = [
    ./caddy-base.nix
  ];

  options.canix-toolbelt.services.caddy.useServiceRegistry = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = "Whether to synthesize Caddy routes from the canix-toolbelt service registry.";
  };

  config = lib.mkIf cfg.useServiceRegistry {
    canix-toolbelt.services.caddy = {
      routes = caddyRoutes;
      cidrExemptHosts = nonCloudflareHostnames;

      # Auto-add caddy-security plugin when any service uses auth.
      plugins = lib.mkIf hasAuthServices (lib.mkBefore [caddySecurityPlugin]);
    };
  };
}
