# nix-topology integration driven by a canix-toolbelt host catalog.
#
# Renders network diagrams from `self.nixosConfigurations` plus a host catalog
# in the canix-toolbelt `hostCatalog` schema (see `lib.hostCatalog`).
#
# The module:
#   - Defines two default networks (`lan` and `wg-home`) — overridable via
#     `networks`.
#   - Auto-derives node interfaces from each host's `network.lanIp` and
#     `network.wgHomeIp`.
#   - Auto-wires WireGuard virtual connections from clients to the server
#     (the host whose `wgHomeIp` ends in `.1`, by default).
#   - Merges a user-supplied `extra` overlay last (physical cabling, routers,
#     internet egress, etc.).
#
# Consumers own the `nix-topology` input; this module reads it via
# `inputs.nix-topology`.
#
# Usage:
#
#   imports = [inputs.canix-toolbelt.flakeModules.topology];
#
#   canix-toolbelt.topology = {
#     enable = true;
#     hosts = import ./lib/hosts.nix;
#     extra = import ./lib/topology.nix;
#   };
#
# Build with:  nix build .#topology.x86_64-linux.config.output
{
  config,
  inputs,
  lib,
  self,
  ...
}: {
  options.canix-toolbelt.topology = {
    enable = lib.mkEnableOption "nix-topology rendering";

    hosts = lib.mkOption {
      type = lib.types.attrsOf (lib.types.attrsOf lib.types.unspecified);
      default = {};
      description = "Host catalog (canix-toolbelt hostCatalog schema).";
    };

    networks = lib.mkOption {
      type = lib.types.attrsOf (lib.types.attrsOf lib.types.unspecified);
      default = {
        lan = {
          name = "Home LAN";
          cidrv4 = "192.168.178.0/24";
        };
        wg-home = {
          name = "WireGuard VPN";
          cidrv4 = "10.123.0.0/24";
        };
      };
      description = "nix-topology network definitions.";
    };

    extra = lib.mkOption {
      type = lib.types.attrsOf lib.types.unspecified;
      default = {};
      description = "Extra topology module merged on top (recursiveUpdate).";
    };

    systems = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = ["x86_64-linux" "aarch64-linux"];
      description = "Systems to emit `flake.topology.<system>` for.";
    };

    wgServerDetector = lib.mkOption {
      type = lib.types.functionTo (lib.types.functionTo lib.types.bool);
      default = _name: hostData: let
        wg = hostData.network.wgHomeIp or null;
      in
        wg != null && lib.hasSuffix ".1" wg;
      description = "Predicate `name: hostData: bool` picking the wg-home server.";
    };
  };

  config = lib.mkIf config.canix-toolbelt.topology.enable {
    flake.topology = let
      tcfg = config.canix-toolbelt.topology;

      nixosConfigurations =
        lib.filterAttrs
        (name: _: builtins.hasAttr name tcfg.hosts)
        self.nixosConfigurations;

      wgServerName =
        lib.findFirst
        (name: tcfg.wgServerDetector name tcfg.hosts.${name})
        null
        (lib.attrNames tcfg.hosts);

      wgClients =
        lib.filterAttrs (
          name: hostData: let
            network = hostData.network or {};
          in
            network ? wgHomeIp && name != wgServerName
        )
        tcfg.hosts;

      topologyModule =
        lib.recursiveUpdate {
          inherit (tcfg) networks;
          nodes =
            lib.mapAttrs (
              name: hostData: let
                network = hostData.network or {};
                hasWg = network ? wgHomeIp && network.wgHomeIp != null;
                isWgServer = name == wgServerName;
              in {
                deviceType = lib.mkForce (hostData.deviceType or "device");
                interfaces = lib.filterAttrs (_: v: v != null) {
                  lan =
                    if network ? lanIp && network.lanIp != null
                    then {
                      network = "lan";
                      addresses = [network.lanIp];
                    }
                    else null;
                  wg-home =
                    if hasWg
                    then
                      {
                        network = "wg-home";
                        addresses = [network.wgHomeIp];
                        type = "wireguard";
                        virtual = true;
                      }
                      // (lib.optionalAttrs isWgServer {
                        physicalConnections =
                          lib.mapAttrsToList (clientName: _: {
                            node = clientName;
                            interface = "wg-home";
                          })
                          wgClients;
                      })
                    else null;
                };
              }
            )
            tcfg.hosts;
        }
        tcfg.extra;
    in
      lib.genAttrs tcfg.systems (system: let
        pkgs = import inputs.nixpkgs {
          inherit system;
          overlays = [inputs.nix-topology.overlays.default];
        };
      in
        import inputs.nix-topology {
          inherit pkgs;
          modules = [
            topologyModule
            {inherit nixosConfigurations;}
          ];
        });
  };
}
