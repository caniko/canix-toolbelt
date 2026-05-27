{
  config,
  lib,
  ...
}: let
  inherit (lib) filter mkOption types;

  reverseProxyServiceSubmodule = types.submodule {
    options = {
      name = mkOption {
        type = types.str;
        description = "Service identifier";
      };
      hostname = mkOption {
        type = types.str;
        description = "Public hostname for reverse proxy";
      };
      port = mkOption {
        type = types.port;
        description = "Port the service listens on";
      };
      targetHost = mkOption {
        type = types.str;
        description = "Hostname where the service runs (key from canix-toolbelt.hosts)";
      };
      proxied = mkOption {
        type = types.bool;
        default = false;
        description = "Whether the service binds to localhost and needs a local reverse proxy";
      };
      cloudflareProxied = mkOption {
        type = types.bool;
        default = true;
        description = "Whether traffic goes through Cloudflare proxy. If false, the hostname is exempt from the Cloudflare CIDR allowlist.";
      };
      publishCname = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Whether the canix.dns synthesizer should emit a public CNAME
          for this service's hostname. Set false for services that are
          reverse-proxied locally but not externally addressable by name
          (admin-only, behind another auth layer, etc.).
        '';
      };
      vpnOnly = mkOption {
        type = types.bool;
        default = false;
        description = "Whether this service is only reachable via VPN DNS (vpn.candee.baby zone).";
      };
      dnsComment = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional provider-side comment for synthesized public DNS records.";
      };
    };
  };

  staticFileServiceSubmodule = types.submodule {
    options = {
      name = mkOption {
        type = types.str;
        description = "Static file service identifier";
      };
      hostname = mkOption {
        type = types.str;
        description = "Public hostname for the static file route";
      };
      staticRoot = mkOption {
        type = types.path;
        description = "Directory served as the static file root";
      };
      cloudflareProxied = mkOption {
        type = types.bool;
        default = true;
        description = "Whether traffic goes through Cloudflare proxy. If false, the hostname is exempt from the Cloudflare CIDR allowlist.";
      };
      vpnOnly = mkOption {
        type = types.bool;
        default = false;
        description = "Whether this service is only reachable via VPN DNS (vpn.candee.baby zone).";
      };
      dnsComment = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Optional provider-side comment for synthesized public DNS records.";
      };
    };
  };
in {
  options.canix-toolbelt.services = {
    sshPort = mkOption {
      type = types.port;
      default = 1337;
      description = "Canonical SSH port for hosts in this registry.";
    };

    hostSshKeyPath = mkOption {
      type = types.str;
      default = "/etc/ssh/id_ed25519";
      description = "Canonical host SSH private key path.";
    };

    hostSshPubKeyPath = mkOption {
      type = types.str;
      default = "/etc/ssh/id_ed25519.pub";
      description = "Canonical host SSH public key path.";
    };

    reverseProxyServices = mkOption {
      type = types.listOf reverseProxyServiceSubmodule;
      default = [];
      description = "Services that need reverse proxy routing and firewall rules";
    };

    staticFileServices = mkOption {
      type = types.listOf staticFileServiceSubmodule;
      default = [];
      description = "Static file services rendered as Caddy file_server routes";
    };

    localServices = mkOption {
      type = types.listOf reverseProxyServiceSubmodule;
      readOnly = true;
      default =
        filter (svc: svc.targetHost == config.networking.hostName)
        config.canix-toolbelt.services.reverseProxyServices;
      description = "Services running on this host (computed from reverseProxyServices)";
    };

    proxiedLocalServices = mkOption {
      type = types.listOf reverseProxyServiceSubmodule;
      readOnly = true;
      default =
        filter (svc: svc.proxied)
        config.canix-toolbelt.services.localServices;
      description = "Local services that need a local reverse proxy (proxied = true)";
    };
  };
}
