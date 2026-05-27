{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.services.initrd-ssh;
  inherit (lib) mkEnableOption mkIf mkOption types;
in {
  options.canix-toolbelt.services.initrd-ssh = {
    enable = mkEnableOption "SSH in initrd for remote rescue access";

    networkInterface = mkOption {
      type = types.str;
      description = "Network interface to configure with DHCP in initrd";
      example = "eno1";
    };

    port = mkOption {
      type = types.port;
      default = 2222;
      description = "SSH port in initrd (use non-standard to avoid host key conflicts)";
    };

    authorizedKeys = mkOption {
      type = types.listOf types.str;
      description = "SSH public keys authorized to connect during initrd";
    };

    hostKeyPath = mkOption {
      type = types.str;
      default = "/etc/ssh/initrd_ed25519";
      description = "Path to the initrd SSH host key (generate with ssh-keygen)";
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = config.boot.initrd.systemd.enable;
        message = "initrd-ssh requires boot.initrd.systemd.enable = true";
      }
    ];

    boot.initrd.systemd.network = {
      enable = true;
      networks."10-${cfg.networkInterface}" = {
        matchConfig.Name = cfg.networkInterface;
        networkConfig.DHCP = "ipv4";
      };
    };

    boot.initrd.network.ssh = {
      enable = true;
      inherit (cfg) authorizedKeys port;
      hostKeys = [cfg.hostKeyPath];
    };
  };
}
