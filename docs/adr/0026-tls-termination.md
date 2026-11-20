<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0026: TLS ends at the operator's reverse proxy

- Status: **Accepted** (decided by the maintainer, 2026-11-20)
- Date: 2026-11-20
- Scope: a decision on paper, no code. Answers
  [TD-19](../RISKS_AND_TECHNICAL_DEBTS.md#td-19) ("No TLS inside the
  services"), which ROADMAP 0.13 asked to decide before 1.0. Part of
  the 0.13 API and configuration freeze ([STABILITY](../STABILITY.md)).

## Context

DSP 2025-1 is bound to HTTPS, and a `did:web` DID is resolved over
HTTPS. So every public address of Portitor must be an `https://`
address. The question is who ends TLS: the three services themselves,
or something in front of them.

What the code does today (checked on 2026-11-20):

| Point | Today | Where |
|---|---|---|
| Listening | Each service listens on plain TCP and speaks plain HTTP, on its public port and on its management port. There is no certificate or key setting for a listener | `crates/portitor-runtime/src/serve.rs` (`bind_management`, `serve`), each `main.rs` |
| Management ports | Bound to `127.0.0.1` by default (8081, 8091, 8071). They serve the health checks without a key and the management API with its key | `DEFAULT_MANAGEMENT_BIND` in `crates/portitor-connector/src/management.rs`, `crates/portitor-identity/src/config.rs`, `crates/portitor-issuer/src/config.rs` |
| Outgoing calls to other participants | TLS with rustls, checked against the system's CA certificates. Only `https://` URLs are called | `crates/portitor-runtime/src/http.rs` (`https_client`, `outbound_url_allowed`) |
| Outgoing calls to the operator's own services | `http://` is allowed for the data sources of the datasets file (`portitor:source`) and for the OTLP receiver: the operator names them | `crates/portitor-connector/src/data_plane.rs` (`HttpDataPlane::new`), DEPLOYING, "Traces and metrics" |
| `PORTITOR_ALLOW_HTTP_CALLBACKS=true` | Also allows `http://` for outgoing calls: callbacks, providers, partners' DIDs and credential services, status lists, the bundle URL and the STS. In the identity and the issuer the same setting allows plain-HTTP DIDs and URLs. For development and tests | same file; each `main.rs` logs a warning at start |
| `PORTITOR_STS_ALLOW_HTTP=true` | Allows a plain-HTTP `PORTITOR_STS_URL` and nothing else: the connector reaches its own identity on the internal network | `crates/portitor-connector/src/auth.rs`, `auth/sts.rs` |
| Public addresses | The operator states them: `PORTITOR_PUBLIC_URL`, `PORTITOR_CREDENTIAL_SERVICE_URL`, `PORTITOR_ISSUER_SERVICE_URL`, and the DID in `PORTITOR_PARTICIPANT_ID`. The services never work them out from a request | `crates/portitor-connector/src/lib.rs`, the identity's and the issuer's `config.rs` |
| Proxy headers | Not read. No code reads `X-Forwarded-*`, `Forwarded` or `Host`, or the caller's network address (searched in `crates/*/src`) | — |
| Telemetry | The HTTP server metric reports `url.scheme` as `http`, because TLS ends before the component | DEPLOYING, "Traces and metrics" |

DEPLOYING already tells operators to run the services behind an HTTPS
reverse proxy, to keep the management ports and `/sts/token` off the
public proxy, and SECURITY-ASSURANCE lists TLS inside the services as
out of scope. TD-19 asked for the decision that makes this the rule for
1.0, or changes it.

## What others do

**Eclipse EDC** (Connector at tag `v1.0.0-RC8`, commit `2a5e560`, read
in a local clone on 2026-11-20). EDC's embedded Jetty can end TLS
itself, as an option:

- `edc.web.https.keystore.path` (not required) and
  `edc.web.https.keystore.type` (default `PKCS12`) name a keystore
  ([`JettyExtension.java`](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC8/extensions/common/http/jetty-core/src/main/java/org/eclipse/edc/web/jetty/JettyExtension.java));
  `edc.web.https.keystore.password` and
  `edc.web.https.keymanager.password` both default to `password`
  ([`JettyConfiguration.java`](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC8/extensions/common/http/jetty-core/src/main/java/org/eclipse/edc/web/jetty/JettyConfiguration.java)).
- With a keystore, every web context (port) listens with TLS; without
  one, every context speaks plain HTTP
  ([`JettyService.java`](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC8/extensions/common/http/jetty-core/src/main/java/org/eclipse/edc/web/jetty/JettyService.java),
  `createConnector`). So the default is plain HTTP, as in Portitor.
- No client certificate setting and no handling of forwarded headers
  was found in that module (searched for `setNeedClientAuth`,
  `setWantClientAuth`, `ForwardedRequestCustomizer` and `X-Forwarded`
  in the whole clone: no match in Java sources).

Not checked: how EDC's own deployments (for example its Helm charts in
other repositories) end TLS in practice.

**Simpl-Open.** Nothing new was read. [ADR 0017](0017-simpl-open.md)
holds what was verified: Simpl-Open agents authenticate each other with
mutual TLS at their Tier 2 gateway, which is a gateway in front of the
connector. mTLS in Portitor is on the roadmap under "Later", as a
Simpl-Open extra.

## Options

1. **TLS stays the operator's job.** The services speak plain HTTP
   behind a TLS-terminating reverse proxy or load balancer. No code.
2. **Optional TLS in the services,** as EDC's keystore setting: new
   settings for a certificate chain and a key per listener, reloading
   on renewal, and tests. Additive, so it can come in 1.x if a user
   needs it.
3. **TLS required in the services.** Breaks every deployment that has a
   proxy today, and still needs a proxy or load balancer for most
   operators (several host names, rate limits, certificates from their
   own tooling).

## Decision

The maintainer decided on 2026-11-20:

1. **TLS stays the operator's job.** The three services speak plain
   HTTP. The operator ends TLS at a reverse proxy or load balancer in
   front of the public ports.
2. **The management ports stay on a private network.** They are never
   routed through the public proxy.

This is option 1, for 1.0 and 1.x. Option 2 stays possible later
without a breaking change; this ADR does not plan it.

## Consequences

- No new setting and no code. TD-19 is Accepted. STABILITY names "no
  TLS in the services" among the things 1.0 does not do.
- What the proxy must do (DEPLOYING has the details per component):
  - serve the public addresses over HTTPS and pass the path unchanged.
    The connector serves `/dsp/2025-1`, `/data` and
    `/.well-known/dspace-version` at the root of its host; the identity
    and the issuer serve their DID document at the DID's path;
  - pass the `Authorization` header and the body unchanged;
  - not buffer or cut the data plane's answers (`GET /data/{flowId}`
    streams, with no size limit);
  - not route `/sts/token` or a management port to the outside.
- The proxy does not need to set `X-Forwarded-*` or `Forwarded`
  headers, and Portitor does not trust them: it reads none. A caller is
  known by its DCP token, never by its network address.
- The path between the proxy and a service is plain HTTP. The operator
  must keep it on a private network (one host, a container network, or
  a network the operator encrypts). Bearer tokens, the management API
  key and the STS client secret cross it.
- Portitor cannot check that TLS is in place. A wrong proxy setup is
  not seen by the service. One check helps: the connector's readiness
  check `identity` reads its DID document at the DID's own URL, as
  partners do, so it fails on a broken proxy, DNS name or certificate
  for that address (DEPLOYING, "Health and logs").
- Client certificates (mTLS) cannot be checked by Portitor. If a data
  space needs them, the proxy checks them; Portitor does not see the
  result.
- `PORTITOR_ALLOW_HTTP_CALLBACKS` and `PORTITOR_STS_ALLOW_HTTP` keep
  their meaning. The first stays for development and tests only.
