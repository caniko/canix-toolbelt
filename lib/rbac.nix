# Role-based access control primitive.
#
# Roles:
#   admin    - userCanReach returns true for every host.
#   personal - userCanReach returns true iff the user is in hostMembership.
#
# Callers provide user data explicitly; this library carries no fleet users.
{
  hosts,
  users ? {},
}: let
  inherit (builtins) attrNames elem filter mapAttrs;

  knownRoles = ["admin" "personal"];

  assertRoles =
    builtins.foldl' (
      acc: u: let
        r = users.${u}.role;
      in
        acc
        && (elem r knownRoles || throw "rbac: user ${u} has unknown role ${r}; expected one of ${toString knownRoles}")
    )
    true (attrNames users);

  userOnHost = userName: hostName:
    hosts.${hostName}.users.${userName}.hasAccount or false;

  hostMembership =
    mapAttrs
    (hostName: _: filter (u: userOnHost u hostName) (attrNames users))
    hosts;

  userCanReach = user: targetHost: let
    inherit ((users.${user} or {role = "personal";})) role;
  in
    role == "admin" || elem user (hostMembership.${targetHost} or []);

  hostsForUser = user:
    filter (h: userCanReach user h) (attrNames hosts);
in
  assert assertRoles; {
    inherit knownRoles users hostMembership userCanReach hostsForUser;
  }
