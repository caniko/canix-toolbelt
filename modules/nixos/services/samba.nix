{
  config,
  lib,
  ...
}: let
  cfg = config.canix-toolbelt.services;
in {
  options.canix-toolbelt.services = {
    samba = {
      enable = lib.mkEnableOption "Samba file server";

      interfaces = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [];
        description = ''
          Network interfaces allowed to reach Samba. When non-empty, the
          system-wide firewall does NOT open Samba ports — instead,
          ports 139/445 (and wsdd discovery) are opened only on the listed
          interfaces, which requires the NixOS firewall. smbd itself
          listens on all addresses: `bind interfaces only` binds once at
          start, so it stays loopback-only when it wins the race against
          DHCP, and Samba never binds non-broadcast interfaces such as
          WireGuard. Empty list (default) opens the ports on every
          interface.
        '';
        example = ["eno1" "wg-home"];
      };

      shares = lib.mkOption {
        type = lib.types.attrsOf (lib.types.submodule {
          options = {
            path = lib.mkOption {
              type = lib.types.str;
              description = "Filesystem path of the share.";
            };
            validUsers = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              description = "Users allowed to access this share.";
            };
            readOnly = lib.mkOption {
              type = lib.types.bool;
              default = false;
            };
            createMask = lib.mkOption {
              type = lib.types.str;
              default = "0644";
            };
            directoryMask = lib.mkOption {
              type = lib.types.str;
              default = "0755";
            };
          };
        });
        default = {};
      };
    };
  };

  config = lib.mkMerge [
    (lib.mkIf cfg.samba.enable (let
      scoped = cfg.samba.interfaces != [];
    in {
      assertions = [
        {
          assertion = scoped -> config.networking.firewall.enable;
          message = "canix-toolbelt.services.samba.interfaces is enforced by the NixOS firewall; enable networking.firewall.";
        }
      ];

      services.samba = {
        enable = true;
        nmbd.enable = false;
        openFirewall = !scoped;
        settings =
          {
            global = {
              workgroup = "WORKGROUP";
              security = "user";
            };
          }
          // lib.mapAttrs (_: share: {
            inherit (share) path;
            browseable = "yes";
            "read only" =
              if share.readOnly
              then "yes"
              else "no";
            "valid users" = lib.concatStringsSep " " share.validUsers;
            "create mask" = share.createMask;
            "directory mask" = share.directoryMask;
          })
          cfg.samba.shares;
      };

      services.samba-wsdd = {
        enable = true;
        openFirewall = !scoped;
      };

      # When scoped, open SMB (445) + NetBIOS-SSN (139) per-interface
      # instead of system-wide. wsdd uses 3702/UDP + 5357/TCP — also
      # scoped so SMB device discovery does not leak past the LAN/VPN.
      networking.firewall.interfaces = lib.mkIf scoped (
        lib.genAttrs cfg.samba.interfaces (_: {
          allowedTCPPorts = [139 445 5357];
          allowedUDPPorts = [3702];
        })
      );
    }))
  ];
}
