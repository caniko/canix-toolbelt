{
  config,
  lib,
  ...
}: let
  caddyLib = import ../../../lib/caddy.nix {inherit lib;};
  cfg = config.canix-toolbelt.services.caddy;
  serviceCfg = config.canix-toolbelt.services;

  reverseProxyRoute = svc:
    caddyLib.mkReverseProxyRoute {
      inherit (svc) hostname;
      port =
        if svc.proxied
        then 80
        else svc.port;
      host = config.canix-toolbelt.hosts.${svc.targetHost}.lanIp;
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
    };
  };
}
