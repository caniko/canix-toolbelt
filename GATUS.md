# Profile-driven Gatus instances

The following NixOS modules compose with Fleetix's service-profile library:

- `gatus-instances`: isolated loopback dashboards, SQLite state, presentation,
  domain selectors, and profile/assignment coverage assertions.
- `gatus-health-publisher`: a host-local timer and hardened service invoking a
  consumer-provided Fleetix-based publisher frontend. The command receives
  `--config` and `--credential-env-file` paths.
- `gatus-external-ingress`: a private IPv4 Caddy listener that permits only
  selected source IPs and POSTs to `/api/v1/endpoints/<key>/external`.

The authenticated dashboard frontend is consumer policy. Keep that gate in front
of the loopback instance; the result-ingestion listener does not serve dashboard
pages or status reads, including to valid bearer-token holders.

Set `services.gatusDiscovery.topology`, `domains`, and `defaultDomain` from the
consumer's topology facade. Instances set `selection.domain`; exactly one may set
`selection.includeInternal = true`. With `requireCompleteProfiles = true`, every
endpoint and site needs a profile and every active check must be assigned exactly
once. Explicit lifecycle exclusions remain inspectable through `excluded`.
Each enabled instance also needs at least one network check: Gatus v5 rejects
external-only configurations at startup, even when external checks are valid.

`client` supplies network-probe defaults such as a private `dns-resolver`;
individual endpoint client settings take precedence. Runtime private CAs belong
in `trustedCertificateFiles`: systemd loads them as credentials, and Go adds
their directory to the system certificate bundle. Hostnames and TLS certificates
are still verified normally. The consuming host must order the instance after
the service that creates those CA files.

Use a runtime `environmentFile` containing `GATUS_EXTERNAL_TOKEN` on the collecting
instance. The same credential is delivered to publishers via
`credentialEnvFile` and systemd `LoadCredential`. The publisher's `checks` are the
instance's rendered `externalChecks`; each host keeps only its own checks.
The result transport should be loopback or an authenticated private network such
as WireGuard. `gatus-external-ingress.peers` uses actual socket source addresses,
not forwarded headers, and its firewall rule is interface- and address-scoped.

Publisher runtime limits include collection/publication batch budgets and timer
accuracy. Configurations whose run limit could overlap heartbeat expiry fail
evaluation. A timed-out process publishes no fresh success, so heartbeat expiry
remains the failure signal. Contract evaluators must honor the supplied timeout;
the systemd limit bounds the entire process as a final backstop.

Preserve `stateDirectory`, display names, and categories when migrating existing
instances. Fleetix's rendered `key` is the key the publisher must use.

## Validation

`nixos-tests/gatus-instances-eval.nix` checks instance isolation, discovery,
credentials, and disabled configurations against a supplied `fleetixLib`.
`nixos-tests/gatus-publisher-eval.nix` checks host selection, heartbeat budgets,
credential delivery, and ingestion routing. These tests can be evaluated against
a local Fleetix library before updating the shared flake input.
