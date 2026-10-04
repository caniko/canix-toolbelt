{pkgs}: let
  python = pkgs.python3.withPackages (p: [p.pillow]);
  browser = pkgs.runCommand "floorp-wrapper-fixture" {} ''
    mkdir -p "$out/bin"
    ln -s ${pkgs.floorp-bin}/lib "$out/lib"
    cat > "$out/bin/floorp" <<'PY'
    #!${pkgs.lib.getExe python}
    import json, os, sys
    with open(os.environ["BROWSER_WRAPPER_RECEIPT"], "w") as receipt:
        json.dump([sys.argv[1:], os.environ["BROWSER_WRAPPER_ENV"]], receipt)
    os.execv(${builtins.toJSON (pkgs.lib.getExe pkgs.floorp-bin)}, [${builtins.toJSON (pkgs.lib.getExe pkgs.floorp-bin)}, *sys.argv[1:]])
    PY
    chmod +x "$out/bin/floorp"
  '';
  settings = {
    browser = {
      family = "firefox";
      # Exercise the selected production wrapper. The metadata-adjacent
      # launcher in browser_connection.py preserves its environment and argv.
      executable = "${browser}/bin/floorp";
      arguments = ["--name" "Toolbelt literal argument"];
    };
    headless = true;
  };
  adapter = (import ../lib/browserConnection.nix {inherit (pkgs) lib;}).mkAdapter {
    inherit pkgs;
    inherit (settings) browser headless;
  };
in
  pkgs.runCommand "browser-connection-floorp-smoke" {
    nativeBuildInputs = [python];
  } ''
    export HOME="$TMPDIR/home"
    mkdir -p "$HOME"
    export BROWSER_CONNECTION_SMOKE_CONFIG=packaged
    export BROWSER_CONNECTION_SMOKE_COMMAND=${pkgs.lib.getExe adapter.package}
    export BROWSER_WRAPPER_RECEIPT="$TMPDIR/wrapper-receipt.json"
    export BROWSER_WRAPPER_ENV='literal environment preserved'
    cp -r ${../runtime} runtime
    mkdir lib
    cp ${../lib/browserConnection.nix} lib/browserConnection.nix
    python -m unittest discover -s runtime -p test_browser_connection.py -v
    touch "$out"
  ''
