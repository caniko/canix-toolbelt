{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.canix-toolbelt.networking.wg-home-client;
  inherit (config.canix-toolbelt.networking) wgHome;
  hostname = config.networking.hostName;
  hostWgHomeIp = config.canix-toolbelt.hosts.${hostname}.wgHomeIp or null;

  # The wg-home server is identified by convention as the unique host with
  # wgHomePublicKey set in the shared host registry. Changing that convention
  # should be discussed at the toolbelt module boundary.
  wgServerCandidates = lib.filterAttrs (_: h: (h.wgHomePublicKey or null) != null) config.canix-toolbelt.hosts;
  wgServerNames = lib.attrNames wgServerCandidates;
  wgServer =
    if wgServerCandidates == {}
    then null
    else lib.head (lib.attrValues wgServerCandidates);
  vpnDnsServer = wgServer.wgHomeIp or null;
  vpnDnsServerForConfig =
    if vpnDnsServer != null
    then vpnDnsServer
    else "0.0.0.0";
  wgServerPublicKey = wgServer.wgHomePublicKey or "";
in {
  imports = [
    ./wg-home-shared.nix
  ];

  options.canix-toolbelt.networking.wg-home-client = {
    clientIp = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default =
        if hostWgHomeIp != null
        then "${hostWgHomeIp}/24"
        else null;
      description = "The IP address for this client on the wg-home VPN, including prefix length.";
    };

    secretName = lib.mkOption {
      type = lib.types.str;
      default = "${hostname}-wg-home-client-pk";
      defaultText = lib.literalExpression ''"${config.networking.hostName}-wg-home-client-pk"'';
      description = "Name of the age secret containing this host's wg-home private key.";
    };

    privateKeyFile = lib.mkOption {
      type = lib.types.str;
      default = config.age.secrets.${cfg.secretName}.path or "";
      defaultText = lib.literalExpression "config.age.secrets.\${config.canix-toolbelt.networking.wg-home-client.secretName}.path";
      description = "Path to the wg-home private key file.";
    };

    enableDns = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Configure split DNS for the wg-home VPN domain via the VPN DNS server.";
    };
  };

  config = lib.mkIf (hostWgHomeIp != null) {
    assertions = [
      {
        assertion = cfg.clientIp != null;
        message = "canix-toolbelt.networking.wg-home-client.clientIp must be set, either directly or via canix-toolbelt.hosts.<hostname>.wgHomeIp";
      }
      {
        assertion = lib.length wgServerNames == 1;
        message = "canix-toolbelt.networking.wg-home-client requires exactly one host with wgHomePublicKey set in canix-toolbelt.hosts";
      }
      {
        assertion = vpnDnsServer != null;
        message = "canix-toolbelt.networking.wg-home-client requires the wg-home server host to have wgHomeIp set";
      }
    ];

    services.resolved.enable = lib.mkIf cfg.enableDns true;

    networking.firewall = {
      allowedUDPPorts = [wgHome.port];
    };

    networking.wireguard.enable = true;
    networking.wireguard.interfaces = {
      wg-home = {
        ips = [cfg.clientIp];
        listenPort = wgHome.port;
        inherit (cfg) privateKeyFile;

        postSetup = lib.mkIf cfg.enableDns ''
          ${pkgs.systemd}/bin/resolvectl dns wg-home ${vpnDnsServerForConfig}
          ${pkgs.systemd}/bin/resolvectl domain wg-home ~${wgHome.vpnDomain}
        '';

        postShutdown = lib.mkIf cfg.enableDns ''
          ${pkgs.systemd}/bin/resolvectl revert wg-home || true
        '';

        peers = [
          {
            publicKey = wgServerPublicKey;
            allowedIPs = ["10.123.0.0/24"];
            endpoint = "${wgHome.endpointHost}:${toString wgHome.port}";
            dynamicEndpointRefreshSeconds = 10;
            persistentKeepalive = 25;
          }
        ];
      };
    };
  };
}
