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
          Network interfaces Samba binds to. When non-empty, the
          system-wide firewall does NOT open Samba ports — instead,
          ports 139/445 are opened only on the listed interfaces, and
          smbd is configured with `bind interfaces only = yes` so it
          refuses connections on unlisted interfaces. Loopback (`lo`)
          is always added. Empty list (default) preserves the legacy
          all-interfaces behaviour.
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
      bindIfaces = ["lo"] ++ cfg.samba.interfaces;
    in {
      services.samba = {
        enable = true;
        nmbd.enable = false;
        openFirewall = !scoped;
        settings =
          {
            global =
              {
                workgroup = "WORKGROUP";
                security = "user";
              }
              // lib.optionalAttrs scoped {
                interfaces = lib.concatStringsSep " " bindIfaces;
                "bind interfaces only" = "yes";
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
