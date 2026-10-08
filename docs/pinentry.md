# Hardware-key pinentry

`homeModules.pinentry` routes GPG/OpenPGP and rage/age-plugin requests using
the requesting context. Enable each integration explicitly:

```nix
{
  imports = [inputs.canix-toolbelt.homeModules.pinentry];
  canix-toolbelt.pinentry = {
    enable = true;
    gpgIntegration = true;
    rageIntegration = true;
    zellij = config.programs.zellij.package;
  };
}
```

| Request | Prompt |
| --- | --- |
| Zellij, including SSH | Pinned floating `Hardware key PIN` pane near the originating pane |
| Local graphical caller outside Zellij | Qt |
| SSH outside Zellij | Requesting terminal |
| Headless | Terminal; fails when no interactive terminal is usable |

Zellij 0.45+ and Linux are required. Live Zellij coordinates take precedence
over inherited desktop markers. Shell hooks and the wrapped `gpg`/`gpg2` classify
requests in Rust; GPG forwards `PINENTRY_USER_DATA` to the shared agent. The
agent entrypoint uses only that per-request metadata, never its startup SSH or
Zellij environment. Direct clients preserve their own display environment.
The desktop marker carries a hex-encoded snapshot of the caller's display,
Wayland socket, Xauthority, session type, runtime directory and Qt platform
selector through GnuPG.
The agent restores that snapshot and clears absent values from older sessions.
The router never replaces it with the systemd user manager's shared environment.
The GPG wrapper captures any already-open terminal, including virtual consoles
and serial TTYs, checking stderr/stdout when stdin is piped. Shell hooks honor
Home Manager's per-shell integration toggles.

The existing `canix-pinentry-v1:zellij:<pane>:<session>`, `:desktop` and `:tty`
markers remain compatible. Background clients can forward the originating
marker when they lack live Zellij coordinates. Session names may contain colons.
Legacy desktop markers retain GnuPG's X11 forwarding. Wrapped GnuPG and rage take
profile precedence over existing raw installations.

## Packaged callers

Home Manager installation alone cannot change the binaries selected by a
packaged application. Use the shared helpers for each such caller:

```nix
let
  prompts = inputs.canix-toolbelt.lib.pinentry.mkPackages {inherit pkgs;};
in {
  # GPG plus request-context capture, and rage plus both pinentry lookup paths.
  runtimeInputs = [prompts.gpg prompts.rage];
}
```

`lib.pinentry.mkRage {inherit pkgs;}` constructs just the wrapped rage package.
Pass `agePackage = p: inputs.canix-toolbelt.lib.pinentry.mkRage {pkgs = p;};`
to agenix-rekey's `configure`. The wrapper sets `PINENTRY_PROGRAM` for secret
requests and prefixes its own executable search path with `pinentry` for
confirmation dialogs. Both settings are command-scoped. Standalone FIDO2
credential generation remains the plugin's own terminal UI.

## Protocol and lifecycle

The private mode-0700 directory/mode-0600 socket conveys only the popup's owned
PTY and holds the pane open. The router rewrites `OPTION ttyname`, including
rage's `/dev/tty`, to that PTY. PIN-bearing Assuan stdout remains directly
connected to the caller. PIN input is hidden and never enters the socket,
arguments, logs or temporary files.

Popup startup has one ten-second deadline. Zellij's exact pre-dispatch
`There is no active session!` discovery failure permits at most three launch
attempts with 100 ms between them, only while no pane helper has connected.
Other failures and successful launches are never replayed. Startup failure can
fall back to the requesting terminal before any Assuan input is consumed.
Closing or timing out an active prompt ends the request; it never falls back or
opens another prompt. Launch diagnostics stay off Assuan stdout, in a private
mode-0600 temporary file removed with the popup's runtime directory.

## Qualification

Production outputs `canix-toolbelt-pinentry` and `rage-pinentry` run installed
integration tests. They exercise disposable GnuPG signing, real Qt/Xvfb and
Zellij, SSH and headless routing, transient discovery recovery, bounded startup
fallback, post-dispatch no-replay, cancellation, timeout,
concurrent requests, and runtime cleanup. A disposable age protocol fixture
named `age-plugin-fido2-hmac` exercises real rage confirmation and secret
callbacks with piped stdin/stdout; its plaintext key stanzas are test-only.
`checks.<system>.pinentry-home-eval` covers opt-in Home Manager composition.

Physical OpenPGP/FIDO2 verification requires the operator's token after
activation. Card PIN caching/FORCESIG and secret identity policy remain with
the consumer.
