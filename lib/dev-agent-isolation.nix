{lib}: {
  mkScopeExecWrapper = {
    pkgs,
    name ? "dev-agent-exec",
    slice,
    targetPath,
  }:
    assert lib.hasSuffix ".slice" slice;
      pkgs.writeShellScript name ''
        exec ${lib.getExe' pkgs.systemd "systemd-run"} --user --scope \
          --slice=${lib.escapeShellArg slice} \
          --quiet --collect --same-dir --expand-environment=no \
          -- ${lib.escapeShellArg targetPath} "$@"
      '';
}
