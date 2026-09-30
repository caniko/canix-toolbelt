{lib}: let
  inherit (lib) types;
  octet = "(0|[1-9][0-9]?|1[0-9][0-9]|2[0-4][0-9]|25[0-5])";
  ipv4 = types.strMatching "${octet}\\.${octet}\\.${octet}\\.${octet}";
  ipv6 =
    types.addCheck (types.strMatching "[0-9a-fA-F:]+")
    (value: (builtins.tryEval (builtins.deepSeq (lib.network.ipv6.fromString value) true)).success);
  ipType = family:
    if family == "ipv4"
    then ipv4
    else ipv6;
  cidrType = family:
    types.addCheck types.str (value: let
      parts = lib.splitString "/" value;
      prefix = lib.last parts;
    in
      builtins.length parts
      == 2
      && (ipType family).check (lib.head parts)
      && builtins.match "(0|[1-9][0-9]*)" prefix != null
      && lib.toInt prefix
      <= (
        if family == "ipv4"
        then 32
        else 128
      ));
in {
  inherit ipv4 ipv6 ipType cidrType;
  hostname = types.strMatching "[a-zA-Z0-9]([a-zA-Z0-9.-]*[a-zA-Z0-9])?";
  interfaceName = types.strMatching "[a-zA-Z0-9_.-]{1,15}";
  runtimeKeyFile =
    types.addCheck (types.strMatching "/(run|var/lib)/[^[:space:]]+")
    (value: !(builtins.elem ".." (lib.splitString "/" value)));
}
