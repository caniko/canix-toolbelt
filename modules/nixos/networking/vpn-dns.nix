# VPN-internal DNS server using dnsmasq.
# Runs on the wg-home hub and resolves the VPN domain for WireGuard clients.
{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.networking.vpn-dns;
  inherit (config.canix-toolbelt.networking) wgHome;
  hostname = config.networking.hostName;
  hostRecord = config.canix-toolbelt.hosts.${hostname} or {};
  hubWgIp = hostRecord.wgHomeIp or null;
  hubCandidates = lib.filterAttrs (_: h: (h.wgHomePublicKey or null) != null) config.canix-toolbelt.hosts;
  hubNames = lib.attrNames hubCandidates;
  isHub = lib.length hubNames == 1 && lib.head hubNames == hostname;
  hubWgIpForConfig =
    if hubWgIp != null
    then hubWgIp
    else "0.0.0.0";
in {
  imports = [
    ./wg-home-shared.nix
  ];

  options.canix-toolbelt.networking.vpn-dns = {
    enable = lib.mkEnableOption "VPN-internal DNS over wg-home";

    domain = lib.mkOption {
      type = lib.types.str;
      default = wgHome.vpnDomain;
      defaultText = lib.literalExpression "config.canix-toolbelt.networking.wgHome.vpnDomain";
      description = "DNS zone served to VPN clients.";
    };

    upstreamServers = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = ["1.1.1.1" "8.8.8.8"];
      description = "Upstream DNS servers for non-VPN queries.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = isHub;
        message = "canix-toolbelt.networking.vpn-dns must run on the unique canix-toolbelt.hosts host with wgHomePublicKey set";
      }
      {
        assertion = hubWgIp != null;
        message = "canix-toolbelt.networking.vpn-dns requires this host to have wgHomeIp set in canix-toolbelt.hosts";
      }
    ];

    services.dnsmasq = {
      enable = true;
      resolveLocalQueries = false;
      settings = {
        # Only listen on the WireGuard interface.
        listen-address = hubWgIpForConfig;
        bind-interfaces = true;

        # Don't read /etc/resolv.conf; upstreams are declared here.
        no-resolv = true;
        server = cfg.upstreamServers;

        # Authoritative for the VPN domain.
        local = "/${cfg.domain}/";
        inherit (cfg) domain;

        address =
          # Service records: *.vpn.example -> hub WG IP. Caddy proxies to backends.
          (map (svc: "/${svc.hostname}/${hubWgIpForConfig}")
            (lib.filter (svc: lib.hasSuffix cfg.domain svc.hostname)
              config.canix-toolbelt.services.reverseProxyServices))
          ++
          # Host records: <hostname>.vpn.example -> host WG IP.
          (lib.mapAttrsToList (name: host: "/${name}.${cfg.domain}/${host.wgHomeIp}")
            (lib.filterAttrs (_: h: (h.wgHomeIp or null) != null) config.canix-toolbelt.hosts));
      };
    };

    networking.firewall.interfaces.wg-home = {
      allowedTCPPorts = [53];
      allowedUDPPorts = [53];
    };
  };
}
