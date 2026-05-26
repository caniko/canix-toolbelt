let
  inherit
    (builtins)
    baseNameOf
    elem
    filter
    hasAttr
    substring
    stringLength
    ;

  defaultIgnorePatterns = [
    ".direnv"
    "result"
    "result-*"
    "target"
    "node_modules"
    "__pycache__"
    ".venv"
    ".tox"
    ".pytest_cache"
    ".mypy_cache"
    ".ruff_cache"
    ".next"
    "dist"
    "build"
    ".cache"
    ".DS_Store"
  ];

  hasPrefix = prefix: str:
    substring 0 (stringLength prefix) str == prefix;

  isAnchoredUnder = root: path:
    root
    != null
    && hasPrefix root path
    && (
      stringLength path
      == stringLength root
      || substring (stringLength root) 1 path == "/"
    );

  mkPeerResolution = {
    hosts,
    selfHost,
    hostname,
    folder,
    peerName,
  }: let
    peer = hosts.${peerName} or null;
    peerKnown = hasAttr peerName hosts;
    selfDirectPeers = selfHost.directLinkPeers or [];
    selfHasWg = (selfHost.wgHomeIp or null) != null;
    localAnchored = isAnchoredUnder (selfHost.dataRoot or null) folder.path;
    remoteBase =
      if localAnchored && (peer.dataRoot or null) != null
      then "${peer.dataRoot}/${baseNameOf folder.path}"
      else null;
    route =
      if
        elem peerName selfDirectPeers
        && (selfHost.directLinkIp or null) != null
        && (peer.directLinkIp or null) != null
      then {
        via = "direct-link";
        sshAlias = "d${peerName}";
      }
      else if (peer.lanIp or null) != null
      then {
        via = "lan";
        sshAlias = "l${peerName}";
      }
      else if (peer.wgHomeIp or null) != null && selfHasWg
      then {
        via = "wg-home";
        sshAlias = "v${peerName}";
      }
      else null;
  in
    if !peerKnown
    then {
      warning = "canix.sync.folders.${folder.id}: peer `${peerName}` is not defined in canix.hosts; skipping.";
      peer = null;
    }
    else if !localAnchored
    then {
      warning = "canix.sync.folders.${folder.id}: path `${folder.path}` is not anchored under ${hostname}'s dataRoot `${selfHost.dataRoot or "null"}`; cannot derive remotePath for peer `${peerName}`, skipping.";
      peer = null;
    }
    else if (peer.dataRoot or null) == null
    then {
      warning = "canix.sync.folders.${folder.id}: peer `${peerName}` has no dataRoot; cannot derive remotePath, skipping.";
      peer = null;
    }
    else if route == null
    then {
      warning = "canix.sync.folders.${folder.id}: peer `${peerName}` is not reachable via direct-link, lan, or wg-home from `${hostname}`; skipping.";
      peer = null;
    }
    else {
      warning = null;
      peer =
        route
        // {
          host = peerName;
          remotePath = remoteBase;
        };
    };

  resolveFolder = {
    hosts,
    hostname,
    selfHost,
    folder,
  }: let
    peerResolutions = map (peerName: mkPeerResolution {inherit hosts selfHost hostname folder peerName;}) folder.peers;
    resolvedPeers = map (r: r.peer) (filter (r: r.peer != null) peerResolutions);
    warnings = map (r: r.warning) (filter (r: r.warning != null) peerResolutions);
  in {
    manifest = {
      inherit (folder) id;
      localPath = folder.path;
      ignorePatterns = defaultIgnorePatterns ++ folder.ignorePatterns;
      peers = resolvedPeers;
    };
    inherit warnings;
  };
in {
  inherit
    defaultIgnorePatterns
    isAnchoredUnder
    mkPeerResolution
    resolveFolder
    ;
}
