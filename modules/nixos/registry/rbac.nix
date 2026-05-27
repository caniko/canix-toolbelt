{
  config,
  lib,
  ...
}: let
  inherit (lib) attrNames concatMapStringsSep elem mkOption types;

  rbacLib = import ../../../lib/rbac.nix;
  knownRoles = ["admin" "personal"];
  inherit (config.canix-toolbelt) users;
  badUsers = builtins.filter (u: !(elem users.${u}.role knownRoles)) (attrNames users);
  rbac = rbacLib {
    inherit (config.canix-toolbelt) hosts;
    inherit users;
  };
in {
  imports = [
    ./hosts.nix
  ];

  options.canix-toolbelt = {
    users = mkOption {
      type = types.attrsOf (types.submodule {
        options.role = mkOption {
          type = types.enum knownRoles;
          description = "RBAC role for this user.";
        };
      });
      default = {};
      description = "User registry for role-based access control.";
    };

    rbac = mkOption {
      type = types.raw;
      readOnly = true;
      default = rbac;
      description = "Computed RBAC outputs: users, hostMembership, userCanReach, and hostsForUser.";
    };
  };

  config.assertions = [
    {
      assertion = badUsers == [];
      message = "canix-toolbelt.rbac: unknown role for users: ${concatMapStringsSep ", " (u: "${u}=${users.${u}.role}") badUsers}; expected one of ${toString knownRoles}";
    }
  ];
}
