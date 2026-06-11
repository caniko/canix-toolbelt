{pkgs}: {
  mkEvalCheck = {
    name,
    assertions,
    runtimeScript ? "",
    resultMessage ? "${name} eval assertions passed",
    nativeBuildInputs ? [],
  }: let
    checkedAssertions =
      builtins.map
      (assertion:
        if assertion.assertion
        then {
          inherit (assertion) name message;
        }
        else throw "${name}: ${assertion.name}: ${assertion.message}")
      assertions;
  in
    pkgs.runCommand name {
      assertionsJson = builtins.toJSON checkedAssertions;
      inherit nativeBuildInputs resultMessage;
    } ''
      mkdir -p "$out"
      printf '%s\n' "$assertionsJson" > "$out/assertions.json"
      ${runtimeScript}
      printf '%s\n' "$resultMessage" > "$out/result"
    '';
}
