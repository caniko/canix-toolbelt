# Hardware-key pinentry

`homeModules.pinentry` routes GPG/OpenPGP and rage/age-plugin requests using
the requesting context. Enable each integration explicitly:

```nix
{
  imports = [inputs.canix-toolbelt.homeModules.pinentry];
  programs.zellij.package = inputs.canix-toolbelt.lib.pinentry.mkZellij {inherit pkgs;};
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
| Desktop or headless with `peers`, local seat locked or absent | The first peer whose seat is in use (below) |

Zellij 0.45+ and Linux are required. Live Zellij coordinates take precedence
over inherited desktop markers. Shell hooks and the wrapped `gpg`/`gpg2` classify
requests in Rust; GPG forwards `PINENTRY_USER_DATA` to the shared agent. The
agent entrypoint uses only that per-request metadata, never its startup SSH or
Zellij environment. Direct clients preserve their own display environment.
The desktop marker carries a hex-encoded snapshot of the caller's display,
Wayland socket, Xauthority, session type, runtime directory and Qt platform
selector through GnuPG.
The agent restores that snapshot and clears absent values from older sessions.
For local requests the router never replaces it with the systemd user manager's
shared environment.
The GPG wrapper captures any already-open terminal, including virtual consoles
and serial TTYs, checking stderr/stdout when stdin is piped. Shell hooks honor
Home Manager's per-shell integration toggles.

### Zellij client and server package

`lib.pinentry.mkZellij {inherit pkgs;}` and
`packages.<system>.canix-toolbelt-zellij` provide the same Zellij cleanup patch.
The standalone pinentry packages and the Home Manager pinentry default use it.
An explicit `canix-toolbelt.pinentry.zellij` override remains authoritative.
Wire that option to `config.programs.zellij.package` as above so the client
commands and the session server use the same corrected package. Existing
sessions retain their running server binary until the owner restarts them.

The patch applies to reviewed Zellij `0.45.0` and `0.45.1` sources. Their
connection-status handler and normal CLI-exit handler both remove a client
before the route-loop epilogue removes it again. Zellij reuses the lowest free
client ID, so a delayed second removal can close a newer connection. The patch
leaves both removals to the common epilogue. The helper rejects an unreviewed
version; review its cleanup paths before extending the version list, or pass
an explicitly verified replacement package.

Upstream source:
[connection status](https://github.com/zellij-org/zellij/blob/efd8fd5a89a20c07a111d248ad7fce53848d2c18/zellij-server/src/lib.rs#L1616),
[CLI exit and route epilogue](https://github.com/zellij-org/zellij/blob/efd8fd5a89a20c07a111d248ad7fce53848d2c18/zellij-server/src/route.rs#L2676).

The existing `canix-pinentry-v1:zellij:<pane>:<session>`, `:desktop` and `:tty`
markers remain compatible. Background clients can forward the originating
marker when they lack live Zellij coordinates. Session names may contain colons.
Legacy desktop markers retain GnuPG's X11 forwarding. Wrapped GnuPG and rage take
profile precedence over existing raw installations.

## Remote seats

A host with its own hardware key can prompt on another host where the user
sits. List commands that reach the other hosts' routers:

```nix
canix-toolbelt.pinentry.peers = [
  ["ssh" "-o" "BatchMode=yes" "-o" "ConnectTimeout=3" "me@laptop" "--" "canix-toolbelt-pinentry"]
];
```

Home Manager writes them to `$XDG_CONFIG_HOME/canix-toolbelt/pinentry-peers`
and installs `canix-toolbelt-pinentry` on the login PATH, so every peer can
serve. The router appends `--serve` to each command.

Peers apply only to requests that have no origin to prompt on: desktop
requests and requests with neither a display nor a terminal (marker
`canix-pinentry-v1:headless`). Zellij and terminal requests keep their origin.
A seat is in use while the user's logind display session is local, active and
unlocked; the screen locker must maintain `LockedHint`.

| Local seat | Desktop request | Headless request |
| --- | --- | --- |
| In use | Local Qt | Local prompt as a peer would show it |
| Locked, absent or unknown | First peer in use, else local Qt | First peer in use, else terminal |

A serving peer with an attached Zellij client opens the floating PIN pane next
to that client's focused pane; otherwise it shows Qt on its user manager's
display. A peer that is unreachable or locked declines before sending its
Assuan `OK` greeting, so the next peer receives an untouched request. Any other
first line, such as a login banner, also declines.
A peer that sends no greeting within 20 seconds is stopped and skipped the
same way. A served request is never forwarded again. When the requester's
stream closes, as on an agent timeout, the peer stops its prompt after one
second.

The forwarded Assuan stream carries the PIN back over the peer command to the
requesting agent and its card. The peer replaces the requester's `ttyname`,
`display`, `xauthority` and `touch-file` options with `NOP`, which keeps
request/reply pairing while dropping paths that only exist on the requester.
Anyone who can run the peer command can therefore show a PIN dialog on that
seat; restrict it to the user's own SSH identity.

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
