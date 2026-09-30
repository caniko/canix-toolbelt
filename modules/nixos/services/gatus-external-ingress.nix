# A private, write-only path for Gatus external health results. Dashboard reads
# remain behind the consumer's authentication frontend, on a different listener.
{
  config,
  lib,
  ...
}: let
  inherit (lib) mkOption types;
  network = import ../../../lib/network-types.nix {inherit lib;};
  cfg = config.services.gatusExternalIngress;
  upstream = config.services.gatusInstances.${cfg.instance};
in {
  imports = [./caddy-base.nix];

  options.services.gatusExternalIngress = {
    enable = lib.mkEnableOption "private write-only Gatus results ingress";
    instance = mkOption {
      type = types.str;
      description = "Enabled, credential-backed gatusInstances name.";
    };
    address = mkOption {
      type = network.ipv4;
      description = "Specific private IPv4 listener address (normally WireGuard).";
    };
    port = mkOption {
      type = types.port;
      description = "Private result listener port.";
    };
    interface = mkOption {
      type = network.interfaceName;
      description = "Authenticated private-network interface carrying submissions.";
    };
    peers = mkOption {
      type = types.listOf network.ipv4;
      description = "Publisher source addresses, checked by Caddy as well as the firewall.";
    };
  };
  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.address != "0.0.0.0" && cfg.peers != [];
        message = "Gatus result ingress needs a specific address and explicit publishers.";
      }
      {
        assertion = upstream.enable && upstream.environmentFile != null && upstream.externalChecks != [];
        message = "Gatus result ingress requires an enabled credential-backed external-check instance.";
      }
      {
        assertion = cfg.port != upstream.port;
        message = "Gatus result ingress and backend ports must differ.";
      }
    ];
    canix-toolbelt.services.caddy = {
      enable = true;
      servers.gatus-health = {
        listen = ["${cfg.address}:${toString cfg.port}"];
        automaticHttps = false;
        metrics = false;
        routes = [
          {
            match = [
              {
                method = ["POST"];
                path_regexp.pattern = "^/api/v1/endpoints/[^/]+/external$";
                remote_ip.ranges = cfg.peers;
              }
            ];
            handle = [
              {
                handler = "reverse_proxy";
                upstreams = [{dial = "127.0.0.1:${toString upstream.port}";}];
              }
            ];
            terminal = true;
          }
          {
            handle = [
              {
                handler = "static_response";
                status_code = 404;
                body = "Not found";
              }
            ];
            terminal = true;
          }
        ];
      };
    };
    networking.firewall.extraInputRules = lib.mkAfter ''
      iifname "${cfg.interface}" ip daddr ${cfg.address} ip saddr { ${lib.concatStringsSep ", " cfg.peers} } tcp dport ${toString cfg.port} accept
    '';
  };
}
