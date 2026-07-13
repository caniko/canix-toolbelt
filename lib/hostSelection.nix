{lib}: let
  isEnabled = policy: hostName: let
    hostPolicy = policy.${hostName} or {};
  in
    hostPolicy.enable or true;

  filterEnabled = policy: attrs:
    lib.filterAttrs (hostName: _: isEnabled policy hostName) attrs;

  unknownHosts = policy: knownHosts:
    lib.filter (hostName: !(lib.elem hostName knownHosts)) (builtins.attrNames policy);
in {
  inherit filterEnabled isEnabled unknownHosts;
}
