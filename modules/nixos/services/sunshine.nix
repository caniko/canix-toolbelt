{
  config,
  lib,
  ...
}: let
  inherit (lib) mkIf mkEnableOption mkOption types genAttrs;
  cfg = config.canix-toolbelt.services.sunshine;
  svc = config.services.sunshine;

  generatePorts = port: offsets: map (offset: port + offset) offsets;
  defaultPort = 47989;
  basePort = if svc.settings ? port then svc.settings.port else defaultPort;

  tcpPorts = generatePorts basePort [(-5) 0 1 21];
  udpPorts = generatePorts basePort [9 10 11 13 21];
in {
  options.canix-toolbelt.services.sunshine = {
    enable = mkEnableOption "Sunshine game stream host for Moonlight";

    openFirewallInterfaces = mkOption {
      type = types.listOf types.str;
      default = [];
      description = ''
        Network interfaces to open Sunshine ports on. When non-empty, the
        system-wide firewall does NOT open Sunshine ports globally — instead,
        ports are opened only on the listed interfaces and the upstream
        `services.sunshine.openFirewall` is forced to `false`.
        Empty list (default) defers to `services.sunshine.openFirewall`.
      '';
      example = ["eno1" "wg-home"];
    };
  };

  config = mkIf cfg.enable {
    services.sunshine.enable = true;

    # When scoped to specific interfaces, force global openFirewall off
    # and open per-interface instead.
    services.sunshine.openFirewall = mkIf (cfg.openFirewallInterfaces != []) false;

    networking.firewall.interfaces = mkIf (cfg.openFirewallInterfaces != [])
      (genAttrs cfg.openFirewallInterfaces (_: {
        allowedTCPPorts = tcpPorts;
        allowedUDPPorts = udpPorts;
      }));
  };
}
