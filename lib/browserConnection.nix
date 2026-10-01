{lib}: {
  mkAdapter = {
    pkgs,
    browser ? null,
    headless ? false,
  }: let
    python = pkgs.python3.withPackages (p: [p.pillow]);
    settings = pkgs.writeText "browser-connection.json" (builtins.toJSON {
      inherit browser headless;
      drivers = {
        firefox = lib.getExe pkgs.geckodriver;
        chromium = lib.getExe pkgs.chromedriver;
      };
      xdgSettings = "${pkgs.xdg-utils}/bin/xdg-settings";
    });
    package =
      pkgs.runCommand "opencode-browser-adapter" {
        nativeBuildInputs = [pkgs.makeWrapper];
        meta.mainProgram = "opencode-browser-adapter";
      } ''
        mkdir -p "$out/lib/browser-connection" "$out/bin"
        cp ${../runtime/browser_connection.py} "$out/lib/browser-connection/browser_connection.py"
        cp ${../runtime/opencode_browser.py} "$out/lib/browser-connection/opencode_browser.py"
        makeWrapper ${lib.getExe python} "$out/bin/opencode-browser-adapter" \
          --add-flags "$out/lib/browser-connection/opencode_browser.py --config ${settings}"
      '';
  in {
    inherit package;
    command = ["${package}/bin/opencode-browser-adapter"];
    # These are implemented through standard WebDriver on both browser families.
    operations = [
      "tabs.list"
      "tabs.open"
      "tabs.focus"
      "tabs.close"
      "navigate"
      "back"
      "forward"
      "reload"
      "stop"
      "frames"
      "snapshot"
      "find"
      "evaluate"
      "click"
      "hover"
      "drag"
      "fill"
      "fill_form"
      "select"
      "check"
      "press"
      "scroll"
      "wait"
      "screenshot"
      "dialog"
      "files.upload"
      "files.drop"
      "files.list"
      "files.get"
    ];
  };
}
