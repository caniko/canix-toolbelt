{lib}: let
  mkTransitionCheck = {
    profile,
    body,
    fromEnabled ? null,
    toEnabled ? null,
    currentDefault ? false,
    currentSystemRoot ? "/run/current-system",
  }:
    assert lib.assertMsg (builtins.match "[A-Za-z0-9._-]+" profile != null) "profiles.mkTransitionCheck: invalid profile name `${profile}`";
    assert lib.assertMsg ((fromEnabled == null) == (toEnabled == null)) "profiles.mkTransitionCheck: fromEnabled and toEnabled must be supplied together";
    assert lib.assertMsg (fromEnabled == null || builtins.isBool fromEnabled) "profiles.mkTransitionCheck: fromEnabled must be a boolean";
    assert lib.assertMsg (toEnabled == null || builtins.isBool toEnabled) "profiles.mkTransitionCheck: toEnabled must be a boolean"; let
      currentValue =
        if fromEnabled == null
        then currentDefault
        else fromEnabled;
      transitionGuard =
        if fromEnabled == null
        then ''[ "$current" != "$next" ] || exit 0''
        else ''[ "$current" = "${lib.boolToString fromEnabled}" ] && [ "$next" = "${lib.boolToString toEnabled}" ] || exit 0'';
      marker = "etc/canix-profiles/${profile}.enable";
    in ''
      incoming="''${1:?missing incoming system path}"
      action="''${2-}"
      [ "$action" = switch ] || exit 0
      set -euo pipefail

      currentMarker="${currentSystemRoot}/${marker}"
      incomingMarker="$incoming/${marker}"
      [ -r "$incomingMarker" ] || exit 0
      current="${lib.boolToString currentValue}"
      [ ! -r "$currentMarker" ] || IFS= read -r current < "$currentMarker"
      IFS= read -r next < "$incomingMarker"
      ${transitionGuard}

      ${body}
    '';
in {
  inherit mkTransitionCheck;
}
