{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.services.publicEdge;
  tunnel = config.canix-toolbelt.networking.edgeTransport;
  udp = builtins.attrValues cfg.udp;
  original = forward: "ct original ip daddr ${cfg.publicAddress} ct original proto-dst ${toString forward.publicPort}";
  target = forward: "ip daddr ${forward.address} udp dport ${toString forward.port}";
in {
  config = lib.mkIf cfg.enable (lib.mkMerge [
    (lib.mkIf (cfg.tcp != {}) {
      services.haproxy = {
        enable = true;
        config = ''
          global
            log stdout format raw local0
          defaults
            mode tcp
            log global
            option tcplog
            timeout connect 5s
            timeout client 1h
            timeout server 1h
          ${lib.concatStringsSep "\n" (lib.mapAttrsToList (name: forward: ''
              listen edge_${name}
                bind ${cfg.publicAddress}:${toString forward.publicPort}
                server origin ${forward.address}:${toString forward.port}${lib.optionalString (forward.proxyProtocol == "v1") " send-proxy"}
            '')
            cfg.tcp)}
        '';
      };
    })
    (lib.mkIf (cfg.udp != {}) {
      boot.kernel.sysctl."net.ipv4.ip_forward" = 1;
      networking.firewall.filterForward = true;
      networking.nftables.tables.public-edge = {
        family = "ip";
        content = ''
          chain prerouting {
            type nat hook prerouting priority dstnat; policy accept;
            ${lib.concatMapStringsSep "\n" (f: ''iifname "${cfg.publicInterface}" ip daddr ${cfg.publicAddress} udp dport ${toString f.publicPort} dnat to ${f.address}:${toString f.port}'') udp}
          }
          chain postrouting {
            type nat hook postrouting priority srcnat; policy accept;
            ${lib.concatMapStringsSep "\n" (f: ''iifname "${cfg.publicInterface}" oifname "${tunnel.interface}" ${target f} ct status dnat ${original f} snat to ${tunnel.address}'') udp}
          }
          chain forward {
            type filter hook forward priority filter - 1; policy accept;
            # NixOS's forward chain accepts all DNAT by default. Enforce the
            # exact edge boundary here before that broader acceptance.
            ${lib.concatMapStringsSep "\n" (f: ''iifname "${cfg.publicInterface}" oifname "${tunnel.interface}" ${target f} ct status dnat ct direction original ${original f} accept'') udp}
            iifname "${tunnel.interface}" oifname "${cfg.publicInterface}" ct direction reply ct state established,related accept
            iifname "${cfg.publicInterface}" oifname "${tunnel.interface}" drop
            iifname "${tunnel.interface}" oifname "${cfg.publicInterface}" drop
          }
        '';
      };
    })
  ]);
}
