{pkgs}: let
  python = pkgs.python3.withPackages (p: [p.pillow]);
  settings = pkgs.writeText "browser-connection-smoke.json" (builtins.toJSON {
    browser = {
      family = "firefox";
      executable = pkgs.lib.getExe pkgs.floorp;
      arguments = [];
    };
    headless = true;
    drivers.firefox = pkgs.lib.getExe pkgs.geckodriver;
  });
in
  pkgs.runCommand "browser-connection-floorp-smoke" {
    nativeBuildInputs = [python];
  } ''
    export HOME="$TMPDIR/home"
    mkdir -p "$HOME"
    export BROWSER_CONNECTION_SMOKE_CONFIG=${settings}
    cp -r ${../runtime} runtime
    mkdir lib
    cp ${../lib/browserConnection.nix} lib/browserConnection.nix
    python -m unittest discover -s runtime -p test_browser_connection.py -v
    touch "$out"
  ''
