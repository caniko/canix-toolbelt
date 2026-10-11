# Headroom coding-agent adapters

Import both `infernix.homeModules.headroom` and
`canix-toolbelt.homeModules.headroom`. Infernix owns the package, compression
service and Fleetix MCP registration; toolbelt composes client transports.

Enable `canix-toolbelt.headroom` and pass the Infernix package as `package`.
For OpenCode, `opencodeGateways` maps existing Jev gateway names to Infernix
`{ url, unit }` endpoints. The original gateway `upstream` remains its client
route key. `forwardUrl` changes only Jev's next transport hop, giving:

```text
OpenCode V2 -> Jev -> Headroom -> existing provider / Meridian
```

Provider IDs, models, OAuth and API-key handling stay with the original
provider. The bundled Headroom V1 OpenCode plugin is not installed.

`codex.enable` installs a higher-priority Codex launcher using the pinned
Headroom 0.40.0 session adapter. `codex.port` identifies its local instance.
It preserves provider IDs, custom-provider URLs and `CODEX_HOME`, and execs
`codex.package` with process-local overrides. This selects Codex embedded
session mode; it does not install, retag sessions, rewrite user configuration
or manage the shared Codex background server. Fleetix owns MCP configuration.

`claude.enable` and `claude.endpoint` set `ANTHROPIC_BASE_URL` while retaining
the user's authentication token and existing Infernix Claude wrapper. Point
that Headroom instance at the Infernix router rather than back at the client.

Disabling this module restores Jev's original upstreams and the unwrapped
Codex package. New login shells pick up the restored Claude environment.
