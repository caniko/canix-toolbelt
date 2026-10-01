# Default-browser connections

`homeModules.browser-connection` supplies browser automation to frameworks
without requiring an Electron application. It discovers the user's XDG default
at connection time and opens an isolated automation profile. Firefox/Floorp use
geckodriver; Chromium-family browsers use chromedriver. An unsupported default
produces an error instead of selecting a different browser.

```nix
{
  imports = [inputs.canix-toolbelt.homeModules.browser-connection];
  canix-toolbelt.browserConnection.enable = true;
  # Set this for SSH sessions or hosts without a graphical display.
  canix-toolbelt.browserConnection.headless = true;
}
```

An explicit override is useful for custom desktop launchers:

```nix
canix-toolbelt.browserConnection.browser = {
  family = "firefox";
  executable = lib.getExe config.programs.floorp.package;
  arguments = [];
};
```

The reusable `lib.browserConnection.mkAdapter { inherit pkgs; }` constructor
returns an adapter package, executable argv, and its native operation inventory.
`canix-toolbelt.browserConnection.adapters.opencode` exposes the same command
and inventory to framework modules. OpenCode's fork supplies the consumer
module `homeModules.browser-connection`, enabled through
`programs.opencode.browserConnection.enable`.

The runtime separates framework-neutral default discovery and WebDriver
lifecycle (`runtime/browser_connection.py`) from OpenCode's native command and
result mapping (`runtime/opencode_browser.py`). Frameworks can reuse the driver
connection or add an adapter using their own protocol. The OpenCode adapter uses
versioned, bounded JSON-lines messages over private process pipes. The framework
owns one process per session and closes it when the session moves, is deleted,
or the plugin unloads. No unauthenticated browser endpoint is published.

Supported operations include tabs, navigation, semantic DOM snapshots using
browser-computed accessible roles/names, input, frames, evaluation, dialogs,
screenshots, uploads/drop and capture retrieval. Only implemented operations
are advertised. Chromium profiling, Lighthouse, traffic/console recording,
download capture and desktop Review previews use OpenCode Desktop's executor.
The browser runs on the backend host, so `localhost` reaches that host.

Run the bounded runtime tests with:

```sh
python3 -m unittest discover -s runtime -p test_browser_connection.py -v
```

The `browser-connection-eval`, `browser-connection-runtime` and
`browser-connection-smoke` flake checks validate disabled/enabled wiring,
discovery/isolation and a real headless Floorp connection. The smoke check
exercises form inputs, click/wait, frames, screenshots and capture retrieval.
