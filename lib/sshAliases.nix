# Host-aware, user-aware SSH settings block generator.
#
# For a given fromHost + user, emits the (target, prefix) aliases that:
#   - are reachable from fromHost (pairwise rules below), and
#   - the user's RBAC role permits.
#
# Prefix rules (target != fromHost throughout):
#   l<name>  iff target.lanIp != null
#   i<name>  same condition as l<name>, port 2222, relaxed host-key, via vthething
#   t<name>  l-condition AND fromHost has wgHomeIp AND target != thething
#   v<name>  fromHost.wgHomeIp != null AND target.wgHomeIp != null
#   d<name>  target in fromHost.directLinkPeers (mutual; asserted symmetric)
#
# Empty result is valid — caller may warn if reachable set is empty.
{
  lib,
  hosts,
  rbac,
  fromHost,
  user,
  defaultIdentity,
  defaultPort,
  overrides ? {},
}: let
  inherit (builtins) attrNames elem filter foldl';

  fromData = hosts.${fromHost} or (throw "ssh/aliases: unknown fromHost ${fromHost}");
  fromHasWg = (fromData.wgHomeIp or null) != null;
  fromPeers = fromData.directLinkPeers or [];

  # Symmetry guard: every directLinkPeers entry must be mutual.
  asymmetricPairs =
    foldl' (
      acc: h: let
        peers = hosts.${h}.directLinkPeers or [];
        bad = filter (p: !(elem h (hosts.${p}.directLinkPeers or []))) peers;
      in
        acc ++ (map (p: "${h} -> ${p}") bad)
    ) []
    (attrNames hosts);

  symmetryOk =
    asymmetricPairs
    == []
    || throw "ssh/aliases: directLinkPeers asymmetric: ${toString asymmetricPairs}";

  mkBlock = addr: extra:
    {
      HostName = addr;
      Port = defaultPort;
      User = "root";
      IdentityFile = defaultIdentity;
      IdentitiesOnly = true;
    }
    // extra;

  # Per-target overrides arrive as a submodule with defaults filled in for
  # every field. Only port/user/identityFile/extraOptions are relevant to the
  # generated settings blocks; hostname/lan/vpn are documentation-only fields
  # consumed by the customBlocks branch and must never leak here. Splice each
  # key only when it differs from its submodule default, so untouched targets
  # don't clobber mkBlock defaults or per-alias overrides (e.g. i<host>
  # Port 2222).
  overrideOf = o:
    (lib.optionalAttrs ((o.identityFile or null) != null) {IdentityFile = o.identityFile;})
    // (lib.optionalAttrs ((o.user or "root") != "root") {User = o.user;})
    // (lib.optionalAttrs ((o.port or defaultPort) != defaultPort) {Port = o.port;})
    // (o.extraOptions or {});

  aliasesFor = targetName: let
    t = hosts.${targetName};
    isSelf = targetName == fromHost;
    isThething = targetName == "thething";
    o = overrideOf (overrides.${targetName} or {});
    hasLan = (t.lanIp or null) != null;
    hasWg = (t.wgHomeIp or null) != null;
    isPeer = elem targetName fromPeers;
  in
    {}
    // lib.optionalAttrs (!isSelf && hasLan) {
      "l${targetName}" = mkBlock t.lanIp ({Compression = false;} // o);
      "i${targetName}" = mkBlock t.lanIp ({
          Port = 2222;
          ProxyJump = "vthething";
          StrictHostKeyChecking = "no";
          UserKnownHostsFile = "/dev/null";
        }
        // o);
    }
    // lib.optionalAttrs (!isSelf && hasLan && fromHasWg && !isThething) {
      "t${targetName}" = mkBlock t.lanIp ({ProxyJump = "vthething";} // o);
    }
    // lib.optionalAttrs (!isSelf && fromHasWg && hasWg) {
      "v${targetName}" = mkBlock t.wgHomeIp o;
    }
    // lib.optionalAttrs (!isSelf && isPeer && (t.directLinkIp or null) != null) {
      "d${targetName}" = mkBlock t.directLinkIp ({Compression = false;} // o);
    };

  reachable = filter (t: rbac.userCanReach user t) (attrNames hosts);
in
  assert symmetryOk;
    foldl' (acc: t: acc // aliasesFor t) {} reachable
