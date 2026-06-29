{
  config,
  lib,
  ...
}: let
  inherit (lib) filter mkOption types;

  authSubmodule = types.submodule {
    options = {
      enable = lib.mkEnableOption "OIDC authentication via caddy-security";

      provider = mkOption {
        type = types.enum ["kanidm" "rauthy"];
        default = "kanidm";
        description = "OIDC identity provider backend.";
      };
    };
  };

  reverseProxyServiceSubmodule = types.submodule {
    options = {
      name = mkOption {
        type = types.str;
        description = "Service identifier";
      };

      auth = mkOption {
        type = authSubmodule;
        default = {enable = false;};
        description = "OIDC authentication configuration for this service route.";
      };
      hostname = mkOption {
        type = types.str;
        description = "Public hostname for reverse proxy";
      };
      port = mkOption {
        type = types.port;
        description = "Port the service listens on";
      };
      upstreamScheme = mkOption {
        type = types.enum ["http" "https"];
        default = "http";
        description = ''
          Transport scheme Caddy uses to dial the upstream. "https" makes Caddy
          re-encrypt to a TLS-terminating backend (e.g. kanidm on 127.0.0.1:8443),
          emitting reverse_proxy transport.protocol="http" with a tls block.
        '';
      };
      tlsServerName = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = ''
          SNI / tls.server_name sent to an upstreamScheme="https" backend.
          Defaults to the public hostname when null.
        '';
      };
      local = mkOption {
        type = types.nullOr types.bool;
        default = null;
        description = ''
          Override loopback dialing. When null (default) it is computed: a
          service whose targetHost equals this Caddy host's networking.hostName
          dials 127.0.0.1 instead of the host lanIp, matching hand-written
          loopback routes (and avoiding localhost→IPv6 mismatches). Set
          true/false to force.
        '';
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
      kind = mkOption {
        type = types.nullOr types.str;
        default = "static";
        description = "Kind of static file service (e.g. 'static-file' for Caddy file_server routes).";
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
