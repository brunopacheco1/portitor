<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0023: Health checks, traces and metrics

- Status: **Accepted** (2026-11-16, by the maintainer)
- Date: 2026-11-15
- Scope: release 0.9, all three components (connector, identity,
  issuer). Closes [TD-18](../RISKS_AND_TECHNICAL_DEBTS.md#td-18) and
  ARCHITECTURE principle 11 ("Observable").
- Decisions already taken by the maintainer (2026-11-15, 0.9 plan): the
  six choices below, each as recommended in the plan, except the demo
  backend, which is an optional Compose file run by hand.

## Context

Today each component answers `GET /health` with `ok` on its public and
management ports and checks nothing. Logs are JSON lines on stdout.
There are no metrics, no traces and no readiness check, so an
orchestrator cannot tell a component that runs from one that can serve,
and the Compose demo only orders the start of its containers.

The images are `FROM scratch` with one static binary: no shell, no
curl. A Docker `HEALTHCHECK` runs inside the container.

What others do (checked 2026-11-15):

- Eclipse EDC's Observability API
  ([README](https://github.com/eclipse-edc/Connector/blob/main/extensions/common/api/api-observability/README.md)):
  `GET /api/check/health`, `/liveness`, `/readiness`, `/startup`; HTTP
  200 when every check passes, 503 otherwise; the body is a
  `HealthStatus` (`spi/core-spi/.../system/health/HealthStatus.java`):
  `{"isSystemHealthy": bool, "componentResults": [{"component": "...",
  "isHealthy": bool, "failure": {"messages": [...], "failureDetail":
  "..."} | null}]}`. EDC's own Dockerfile example calls
  `curl --fail .../api/check/health`.
- OpenTelemetry: OTLP over HTTP; the HTTP semantic conventions name the
  request duration histograms `http.server.request.duration` and
  `http.client.request.duration`; configuration through the `OTEL_*`
  environment variables. Prometheus 3 accepts OTLP with
  `--web.enable-otlp-receiver`
  ([guide](https://github.com/prometheus/docs/blob/main/docs/guides/opentelemetry.md)).
- The 0.9 spike ([report](../spikes/0.9-opentelemetry.md)): the
  OpenTelemetry Rust crates 0.33 fit the footprint budget, with two
  fixes (exports run on tokio; no `hyper` feature).

## Decision

### 1. Health checks: EDC's Observability API

- Each component serves `GET /api/check/liveness`, `/readiness`,
  `/startup` and `/health` on its **management port, without the API
  key**, with EDC's status codes and `HealthStatus` body. The body may
  name which check failed and why, never secrets or values; the
  management port faces the operator's private network, not other
  participants.
- The management listener now always starts. Without
  `PORTITOR_MANAGEMENT_API_KEY_FILE` it serves only these four checks;
  with it, also the management API as before.
- Liveness checks nothing but the process (it answers). Startup and
  health equal readiness, as in EDC.
- Readiness checks only the component's **own** dependencies, never
  other participants, so a partner's outage never restarts a healthy
  process:
  - connector: the SQLite file answers; the start harvest ran; the
    bundle is in force (when one is configured); its own DID document
    resolves and its STS signs (the start check that exists since 0.5).
    A background task runs this identity check every 10 s with a 3 s
    timeout, and probes read its last result: the check fails only
    after 2 failures in a row (about 20 s), passes again after one
    success, and is not ready before its first result;
  - identity, in the process: the SQLite file answers; the key is loaded
    and signs; the DID document it serves names the participant and
    lists that key;
  - issuer, in the process: the SQLite file answers; the key is loaded
    and signs; both status lists it serves are signed by that key.
- `GET /health` on the public port stays as it is (`ok`).

### 2. A `health` subcommand for Docker

`portitor-connector health` (and the same for identity and issuer)
calls its own `/api/check/readiness` on the management bind address and
exits 0 when it answers 200, 1 otherwise. The Dockerfile sets
`HEALTHCHECK` with it, and the Compose demo starts each component only
when the ones it needs are healthy (`depends_on: condition:
service_healthy`), so the Compose file shows the deployment order.

### 3. OpenTelemetry traces and metrics, off by default

- Export is on only when `OTEL_EXPORTER_OTLP_ENDPOINT` is set. Protocol:
  OTLP over HTTP with Protocol Buffers. Configuration through the
  OpenTelemetry variables the Rust SDK reads (listed in the spike
  report and DEPLOYING); no new `PORTITOR_*` variables.
- Exports are sent with Portitor's own HTTPS client (rustls, ring), as
  a task on the tokio runtime. `opentelemetry-otlp`'s `hyper-client`,
  `reqwest-*` and `grpc-tonic` features stay off.
- The OpenTelemetry layer has its own level filter (`info`), separate
  from `RUST_LOG`, so the log level does not decide what is traced.
- With export on, every JSON log line inside a span carries its
  `trace_id`.

### 4. Trace context stays inside one participant

W3C Trace Context (`traceparent`) is sent and read only on calls inside
one participant: connector to its identity (STS) and connector to its
data source. DSP calls to other participants carry no `traceparent`,
and one that arrives on the DSP port is ignored; each side starts its
own trace. A trace id from another organisation is untrusted input,
and ours would link our internal calls to theirs. Across participants,
both sides log the DSP process ids, which is enough to match their logs.

### 5. Metrics

- HTTP: `http.server.request.duration` and `http.client.request.duration`
  (seconds, histograms) with the semantic conventions' attributes
  (`http.request.method`, `http.response.status_code`, `http.route`,
  `server.address` for client calls). No URL paths with ids, no query
  strings, no headers.
- Portitor: a short list under `portitor.`, following OpenTelemetry's
  naming rules, checked against the semantic conventions before use:
  negotiations and transfers ended (by role and final state), DSP
  requests refused (by reason), outbox resends and give-ups, harvest runs
  (by outcome) and rejected datasets, and the bundle version in force.
  The exact names are listed in DEPLOYING.
- Span and metric attributes are an allow-list: never tokens,
  credentials, keys, request or response bodies, or dataset contents.

### 6. An optional observability stack for trying it out

`examples/compose/observability.yaml`, used together with the demo
Compose file, adds an OpenTelemetry Collector, Jaeger and Prometheus,
pinned by digest. It is started by hand and described in TESTING; CI
does not run it. CI proves export with a test that uses an in-process
OTLP receiver.

### Changes made while building (2026-11-15)

- The health checks get no span and no measure: probes every few
  seconds would fill traces and metrics with noise. The connector's
  background identity check is not traced and sends no `traceparent`.
- Server spans carry no `url.path` and client spans no `url.full`, as
  paths and URLs hold process ids and tokens. Server spans and the
  server metric carry `http.route` (the route template); client spans
  carry `server.address` and `server.port`.
- The client metric carries `server.address` and `server.port` only for
  calls inside the participant (its own STS and data sources). Other
  hosts come from partners' DIDs and callback addresses, which are
  untrusted input and would give the metric unbounded cardinality.
- The identity joins an incoming `traceparent` on `/sts/token` only
  after the client is authenticated, so an unauthenticated caller
  cannot join our traces.
- Each negotiation and transfer state change is also one INFO log line
  (target `portitor::state`), not only a span event.
- The connector's identity check runs in the background (every 10 s,
  3 s timeout, not ready after 2 failures in a row), instead of in the
  probe, so a slow STS never makes a probe slow.
- `OTEL_*` variables with no effect: `OTEL_SDK_DISABLED`,
  `OTEL_EXPORTER_OTLP_PROTOCOL` (the protocol is fixed),
  `OTEL_BSP_EXPORT_TIMEOUT`, `OTEL_BSP_MAX_CONCURRENT_EXPORTS` and
  `OTEL_METRIC_EXPORT_TIMEOUT` (only the SDK's experimental async
  processors read them). `OTEL_EXPORTER_OTLP_COMPRESSION` is read, but
  compression is not built in: the SDK logs a warning and sends
  uncompressed.
- `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` and
  `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` switch export on too, each for
  its own signal; `OTEL_EXPORTER_OTLP_ENDPOINT` counts for both.

## Consequences

- Release build against 0.8.0 (with the health checks): binaries +14%
  (connector), +20% (identity, issuer); idle memory +1.2 MB per process
  with export off, +2.8 MB with it on. At saturation 8% fewer requests
  per second with export off, 12% with every request traced, 18% with
  one trace in ten; runs vary by up to 15%. Sampling lowers the data
  sent, not the cost in the process (DEPLOYING, R-37).
- The management port is now always bound (default `127.0.0.1`); in a
  container, operators who want orchestrator probes set
  `PORTITOR_MANAGEMENT_BIND` to an internal address, as for the
  management API.
- The OpenTelemetry Rust crates are 0.x and change their API with most
  minor releases. All OpenTelemetry code lives in one module of
  `portitor-runtime`, and Dependabot groups their updates.
- A cross-participant trace is not possible by design; debugging across
  organisations goes through the DSP process ids.
- OpenTelemetry logs export, dashboards and alert rules are not part of
  this decision.
