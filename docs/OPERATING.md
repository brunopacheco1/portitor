<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Operating Portitor

For the operations team, after the deployment
([DEPLOYING](DEPLOYING.md)): health checks, logs, traces and metrics,
backups, key changes and upgrades.

## Health and logs

- `GET /health` on the public port (and on the management port, with the
  management API on) answers `ok` while the process runs. It checks
  nothing.
- **Health checks** (ADR 0023), the same in all three components, on the
  **management port**, without the API key, also when the management API
  is off. The answer is Eclipse EDC's `HealthStatus`: HTTP 200 when every
  check passes, 503 otherwise
  ([OpenAPI](api/management-openapi.json), tag `Health`):

  ```json
  {"isSystemHealthy": false, "componentResults": [
    {"component": "database", "isHealthy": true, "failure": null},
    {"component": "identity", "isHealthy": false,
     "failure": {"messages": ["STS: HTTP 401 Unauthorized"], "failureDetail": "STS: HTTP 401 Unauthorized"}}]}
  ```

  | Path | Checks |
  |---|---|
  | `GET /api/check/liveness` | nothing but the process (component `process`) |
  | `GET /api/check/readiness` | the component's own dependencies, never a partner (below) |
  | `GET /api/check/startup`, `GET /api/check/health` | the same as readiness |

  Readiness of the connector:
  - `database`: the database answers (PostgreSQL, or the SQLite
    file). <!-- 1.3: check after integration -->
  - `harvest` (the start harvest ran) and `bundle` (a version is in
    force; only with a bundle): start gates. The connector binds its
    ports only after both, so they cannot fail while it serves.
  - `identity`: the connector's DID document lists a credential service,
    and its STS signs a token that the document's key verifies. The
    document is read at the DID's own URL, as partners read it, so this
    check also goes through your reverse proxy, DNS and certificate. A
    background task runs it every 10 s, with a 3 s timeout; probes read
    the last result. It fails only after 2 failures in a row (about
    20 s), and passes again after one success. Before its first result
    the connector is not ready.

  Identity: `database`, `key` (the key is loaded and signs),
  `did-document` (the DID document it serves names the participant and
  lists that key). Issuer: `database`, `key`, `status-lists` (both
  status lists it serves are signed by that key). These run in the
  process, at each probe. Each check gets 0.5 s. A failure names the
  check and why, never a key or token. A check is logged only when its
  result changes: `readiness check failed` (WARN, with `component` and
  `reason`) when it starts failing, `readiness check passes again`
  (INFO) when it recovers. The health checks are neither traced nor
  measured, and the background identity check sends no `traceparent`.
- **`health` subcommand**: `portitor-connector health` (and
  `portitor-identity health`, `portitor-issuer health`) calls its own
  `GET /api/check/readiness` on `PORTITOR_MANAGEMENT_BIND` (`0.0.0.0` is
  called as `127.0.0.1`), prints the answer, and exits 0 on HTTP 200, 1
  otherwise or after 3 s. It needs the same `PORTITOR_MANAGEMENT_BIND` as
  the running process.
- **Docker**: each image has a `HEALTHCHECK` with that subcommand
  (every 30 s, 5 s timeout, 3 retries; every 2 s during a 60 s start
  period). `docker ps` shows `healthy` or `unhealthy`; Docker Compose can
  start a service once another is healthy (`depends_on: condition:
  service_healthy`, as in `scenarios/two-party/compose.yaml`). The check
  every 2 s during the start period (`--start-interval`) needs Docker
  Engine 25 or later (Docker Desktop 4.27 or later). Podman runs the
  images too, but may not read their `HEALTHCHECK`: see
  [TESTING](TESTING.md#docker-or-podman).
- **Kubernetes**: set `PORTITOR_MANAGEMENT_BIND=0.0.0.0:<port>` and probe
  the management port (8081 connector, 8091 identity, 8071 issuer); keep
  that port out of any `Service` that the public reaches:

  ```yaml
  startupProbe:
    httpGet: {path: /api/check/startup, port: 8081}
    periodSeconds: 2
    failureThreshold: 60   # the start check of the identity may take ~30 s
  livenessProbe:
    httpGet: {path: /api/check/liveness, port: 8081}
  readinessProbe:
    httpGet: {path: /api/check/readiness, port: 8081}
    periodSeconds: 10
  ```

  A failing readiness takes the pod out of its `Service`; it does not
  restart it. Only liveness restarts, and it checks only the process.
- Logs are JSON lines on stdout, one event per line. Collect them with
  your platform's log driver.

The connector's log lines are below; the identity's and the issuer's
have their own tables ([Identity logs](#identity-logs),
[Issuer logs](#issuer-logs)). All three also log, at start,
`management port listening` (INFO, with the address as `bind`) and, with
export on, `telemetry: exporting` (INFO).

| Log message (level) | Meaning | What to do |
|---|---|---|
| `counter-party not reachable; terminating` (WARN) | A message to a partner failed and the partner's state does not show it arrived; the negotiation or transfer is terminated. The message is tried up to 4 times (0.5 s, 1 s, 2 s apart) when no answer came, or the answer was 408, 429 or 5xx; any other answer (400, 404, 409, ...) is final after one try, and `error` shows it | No answer: check the partner's address, DNS, TLS trust and your egress rules. An HTTP status: the partner refused the message; its logs say why |
| `message breaks the dataspace's message rules; not sent, terminating` (WARN) | With bundle message shapes: our own message failed them (`error` lists the SHACL reasons), so it was not sent and the negotiation or transfer is terminated; `..., process not started` for a consumer's first request, `termination breaks ...; not sent` for a termination | Check the offer or policy against the authority's rules |
| `outbound DSP message breaks the dataspace's message rules; not sent` (WARN) | The same breach, logged by the check itself with `message_type` and `reasons` | As above |
| `inbound DSP message breaks the dataspace's message rules; answered 400` (INFO) | A partner's negotiation or transfer message failed the bundle's message shapes; it got HTTP 400 and nothing changed | Nothing, unless the partner asks |
| `inbound DSP message is not valid; answered 400` (INFO) | A partner's negotiation or transfer message could not be read; `reason` says why. `unknown field ...` in an offer or agreement: a policy term the connector does not read (for example a `duty` inside a permission), refused on purpose. The partner got HTTP 400 and nothing changed | Nothing, unless you want that offer: then ask the partner for one with only the terms listed under [Policies](DEPLOYING.md#policies) |
| `transfer request is refused; answered 400` (INFO) | A partner asked for a transfer under an agreement that is not finalized here, is not its own, or does not exist (`reason`: `no finalized agreement with this id for the caller`). It got HTTP 400 and no transfer was made. The line names neither the agreement nor the partner. A request that arrives just before the partner's acknowledgement of the finalization is recorded is accepted, not refused (ADR 0007) | Nothing, unless a partner asks: its negotiation must reach `FINALIZED` on your side first (look for its `negotiation state changed` lines) |
| `message had arrived before` (INFO) | A send failed, but the partner's state shows it had received the message (a lost answer, or a resend after a restart); the negotiation goes on | Nothing |
| `sending pending negotiation messages again` (INFO, at start) | Messages saved but not acknowledged before the last stop are being sent | Nothing |
| `counter-party not reachable; process not started` (WARN) | The first request of a negotiation or transfer we started as consumer failed (tried as above); it ends without a message, as the provider never knew it | Same as above |
| `termination not acknowledged` (WARN) | The partner did not take a termination; DSP allows this, the negotiation is terminated anyway | Nothing, unless it repeats for one partner |
| `no CA certificates: https calls will fail` (WARN, at start) | The CA bundle was not found (for example `SSL_CERT_DIR` points to an empty folder) | Fix the mount or the variable |
| `negotiation store failed` (ERROR) | The database could not be read or written; partners get HTTP 500 | PostgreSQL: check that the server answers and the readiness check `database`. SQLite: check disk space and the `/data` volume |
| `DSP request refused: caller not authenticated` (WARN) | A DSP request was refused: the caller got HTTP 404 on a negotiation or transfer route, HTTP 401 on a catalog route (`status` says which). `reason` says why (token, presentation, credential or status list); the caller is not told | If a partner should be admitted: check its credential, its issuer against `PORTITOR_TRUSTED_ISSUERS`, and clock skew (60 s allowed) |
| A partner says a call got HTTP 404 `unauthorized`, or its side ended with `no such negotiation` although the process exists | The 404 for a caller that is not authenticated does not say why (DSP 2025-1; Eclipse EDC does the same) | Look in **your** log for the line above at that time. A 404 with reason `no such negotiation` or `no such transfer` and without that line means the process id is unknown, or belongs to another participant |
| `counter-party not reachable; terminating` (WARN) with `HTTP 404` in `error` | The partner does not know the process, or it refused your credentials (a Portitor from 0.12 and Eclipse EDC answer 404 for both). The message is not sent again | Ask the partner for the reason in its log; check that your credential is valid and not revoked, and that the partner trusts its issuer |
| `DSP request not checked: a service the check needs is unavailable; answered 503` (WARN) | A server the check fetches from (the partner's DID document or Credential Service, its issuer's DID document or status list, your STS) did not answer, or answered 408, 429 or 5xx; `reason` names it. The partner tries again | If it repeats: check your STS, then ask the partner about its services |
| `DSP termination accepted from a partner without membership` (WARN, 1.1) | A partner whose DCP token is valid but whose membership no longer holds (revoked, expired, untrusted issuer, missing) ended a transfer it is the counter-party of; `process` and `caller` name them, `reason` says what failed. Every other message from it is refused (TD-47) | Nothing: the transfer ended as the partner asked. Check whether the partner is still meant to be in the data space |
| `data flow ended: the consumer's membership no longer holds` (WARN, 1.1) | As provider: the consumer's `MembershipCredential` kept with the data flow failed its check again (`reason`); its request was refused with 401 or its download cut, and the transfer is terminated (the DSP termination goes to the consumer) | Nothing, unless the consumer should still be a member: then check its credential at the issuer |
| `request for data refused: the consumer's membership could not be checked now` (WARN, 1.1) | A `/data` request got 401 because the membership check could not finish (`reason`: a status list or the issuer's DID document did not answer). The transfer is kept; the next request may pass | If it repeats: check the issuer's status list and DID document from the connector's network |
| `running download cut: the consumer's membership could not be checked now` (WARN, 1.1) | The same, for a download that was running: it was cut, the transfer is kept | As above |
| `running downloads cut` (INFO, 1.1) | A data flow was suspended or terminated while downloads of it ran (`downloads` counts them, `state` is the new state); they were stopped | Nothing |
| `data flow started before 1.1: no membership kept; served until its token expires` (INFO, 1.1) | Once per flow and process: a transfer started before the upgrade has no membership to check again; it is served as in 1.0 until its token expires | Nothing; it stops once those tokens have expired (`PORTITOR_DATA_TOKEN_SECONDS`) |
| `the consumer's memberships could not be kept; its requests for data will be refused` (ERROR, 1.1) | The database could not be written when the transfer was asked for; the transfer is refused at the data plane (fail closed) | Check disk space and the `/data` volume; the consumer asks for a new transfer |
| `transfer terminated by the provider's operator` (INFO, 1.1) | `POST /v1/provided-transfers/{providerPid}/termination` was called: access ended and the termination is on its way | Nothing |
| `transfer not ended again` (INFO, 1.1) | A transfer whose consumer is no longer a member had already ended, or its termination was on its way | Nothing |
| `no token: ...` in a send failure (WARN) | The STS did not give a token for an outgoing DSP request | Check the identity component, `PORTITOR_STS_URL` and the shared secret |
| `own identity checked: DID document and STS agree` (INFO, at start) | The connector's DID document resolves and the STS signs for it | Nothing |
| `panic` (ERROR) | A bug: code stopped at `place` (file, line and column). One of the next two lines follows | Report it with `place` and the version |
| `a request handler panicked; answered 500` (ERROR) | The request on `route` got HTTP 500; the other requests went on | Report it; if it repeats for one partner, its message triggers the bug |
| `a background task panicked; it starts again` / `...; its message is sent again` / `...; its work is left as saved` (ERROR) | The task `task` stopped at a bug. A task that runs all the time starts again after a pause (`restart_in_seconds`): 5 seconds, twice as long after each further panic in a row, at most 5 minutes. A DSP message whose sending panicked (`outbox_delivery`) is sent again after the same pauses (`again_in_seconds`). A credential decision stays saved and is tried again on the issuer's next outbox round (at most a minute); the other requests and deliveries go on | Report it. A line that repeats for the same `pid` or `issuer_pid` is a message or request that meets the bug every time: it stays as it is until a fixed version runs |
| `portitor-connector stopped` (ERROR) | The process could not start or stopped with an error; the message says why | Fix the cause; the container restarts |
| `data transfer done` (INFO, target `portitor::audit`) | One per data response served at `/data/{flowId}`, per push sent and per push received (since 1.1), when it ended: the audit line below | Ship and keep it as your audit trail |
| `upstream fetch failed` (WARN) | The data source of a pull did not answer, or answered other than 2xx; the consumer got HTTP 502 and `error` says why. The audit line says `failed` | Check the dataset's `portitor:source` and that the source is up. From 1.2 a transfer reads the source it started with: a corrected `portitor:source` reaches only new transfers |
| `object storage` (INFO, at start, 1.1) | The connector has object storage: `endpoint` and `push_bucket` (`none` when it does not receive pushes); never the keys | Nothing |
| `push: the source could not be read` (WARN, 1.1) | As provider: the source of a push did not answer, or answered other than 2xx (`error`); the push is not sent and the transfer is terminated | Check the dataset's `portitor:source` and that the source is up. From 1.2 a corrected `portitor:source` reaches only new transfers |
| `push not sent: the consumer's membership could not be checked now` (WARN, 1.1) | As provider: the membership check before or during a push could not finish (`reason`); the push fails and the transfer is terminated | Check the issuer's status list and DID document from the connector's network; the consumer asks for a new transfer |
| `push: cannot start; the transfer is terminated` (WARN, 1.1) | As provider: a started push transfer has no source kept (only one started under 1.1 whose dataset had no `portitor:source` at the upgrade to 1.2), or the database failed (`error`); nothing is sent and the transfer is terminated | Check the `/data` volume. For a missing source: the consumer asks for a new transfer |
| `push: the end of the transfer could not be saved` / `push: the started pushes cannot be read` (ERROR, 1.1) | As provider: the database failed while a push ended, or at start while looking for pushes to send again | Check disk space and the `/data` volume |
| `transfer not taken up: its data plane step stays recorded and runs again at its next change, the next sweep or the next restart` (ERROR, 1.2) | As provider: at start or in the sweep (every 10 s), the database failed while running a data plane step left over (`transfer`, `failure`). The connector goes on; the step runs again later | Check disk space and the `/data` volume |
| `data plane step left over ran; its lease had ended` (INFO, 1.2) | As provider: a data plane step recorded with a transfer's change had not run (its process stopped, or recording it failed); once its 30-second lease ended, the start or the sweep ran it (`transfer`), and the message that waited for it is sent | Nothing, unless it repeats: then look for the reason the step was left (a crash, or database errors) |
| `transfer sweep: the transfers cannot be read` (ERROR, 1.2) | As provider: the sweep for steps left over could not read the database; it tries again in 10 seconds | Check disk space and the `/data` volume |
| A partner says its transfer message got HTTP 503 with reason `the transfer is being changed; retry` (1.2) | Another call (in this process, or another one on the same database) was changing that transfer at that moment; nothing was written, and the answer carries `Retry-After: 1`. Eclipse EDC retries such answers. `POST /v1/provided-transfers/{providerPid}/termination` answers the same with a problem document | Nothing; retry. If it lasts longer than 30 seconds for one transfer, look for the ERROR lines above |
| `push received: the object storage failed` (WARN, 1.1) | As consumer: a push arrived but could not be stored (`error` names a status or a cause, never a key); the sender got HTTP 502. The audit line says `failed` | Check the object storage, the push bucket and the connector's key |
| `object storage: an unfinished upload could not be aborted` (WARN, 1.1) | An upload into the push bucket failed, and its parts could not be removed | Remove unfinished multipart uploads with your storage's tools, or set a lifecycle rule for them |
| `harvesting datasets` (INFO, at start) | The datasets file and the interval in use | Nothing |
| `no datasets file (PORTITOR_DATASETS_FILE): the catalog is empty` (INFO) | No datasets file is set | Set it, if you offer datasets |
| `harvest run` (INFO) | One per run: `job`, `trigger`, `outcome` and the counts | Nothing |
| `harvest run` (WARN, `outcome=failed`) | The source could not be read; the catalog is unchanged | Check the file, its mount and its JSON |
| `harvest: dataset rejected` (WARN) | One dataset failed a check: `position`, `dataset`, `check`, `error`. With check `odrl` and `offer "..." is not understood: unknown field ...`: the offer has a policy term the connector does not read, or a misspelled key | Fix the dataset in the source; the next run takes it. For an unknown field: correct the key, or remove the term ([Policies](DEPLOYING.md#policies)) |
| `harvest: dataset has no portitor:source; its transfers will be refused` (WARN) | A dataset was added or changed without a data source | Add `portitor:source`, unless the dataset is not transferred |
| `harvest skipped: a run is going on` (INFO) | A timer tick came while a run was going on | Nothing, unless it repeats: then lengthen the interval |
| `harvest: cannot store the run; ...` (ERROR) | The database could not be written; the catalog keeps what it had | Check disk space and the `/data` volume |
| `looking for new bundle versions` (INFO, at start) | The bundles URL or folder and how often `latest.json` is read | Nothing |
| `bundle: no new version is looked for (...)` (INFO, at start) | A fixed `PORTITOR_BUNDLE_VERSION`, or `PORTITOR_BUNDLE_REFRESH_SECONDS=0` | Nothing |
| `bundle file skipped: this version does not know its role; what the file holds is not enforced` (WARN) | The bundle in force lists a file with a role of a later version (`path`, `role_type`, `version`). The file is not downloaded and not used; the other files are. Logged each time a version is taken | Upgrade the connector to enforce that kind of rule; until then it is not enforced here. If the role type looks misspelt, tell the authority |
| `bundle downloaded and verified` (INFO) | A bundle version was downloaded and passed every check | Nothing |
| `bundle download failed; using the cached copy, checked again` (WARN, at start) | The authority's host or DID document could not be reached; the cached copy is used | Check the network to the authority; nothing breaks meanwhile |
| `bundle: the newest version cannot be taken; keeping the version in use` (ERROR, at start) | With `latest`: the newest version could not be downloaded, failed a check or is older than the one in use; the cached version in use before is kept | Check the reason in the log line or in `GET /v1/harvest-jobs`; ask the authority |
| `bundle: no new version taken; keeping the version in use` (WARN) | A scheduled check failed for the same reasons; nothing changed | As above; the next check tries again |
| `bundle: new version in use; checking the stored datasets again` (INFO) | A newer version was put in use without a restart | Nothing; watch the next harvest jobs for hidden datasets |
| `bundle verified, but not written to the cache` (WARN) | The cache folder is not writable | Check `PORTITOR_BUNDLE_CACHE_DIR` and the volume |
| `bundle rules in force` (INFO, at start) | The bundle's `version`, `profile`, the `access_rights` it has dataset shapes for, and whether it has `message_shapes` | Nothing |
| `bundle shapes: ignored by the SHACL engine` (INFO or WARN, at start) | A SHACL term the engine ignores (`sh:extends`, `sh:shape`, `sh:property` without `sh:path` at INFO: expected for HealthDCAT-AP and DCAT-AP; anything else at WARN) | Nothing at INFO; at WARN, ask the authority |
| `bundle context: not used` (WARN, at start) | The bundle has a `context` file for a DSP 2025-1 context URL; the connector's own copy is used | Nothing |

### Identity logs

| Log message (level) | Meaning |
|---|---|
| `portitor-identity listening` (INFO) | Started; shows the DID, the key id and the database |
| `retired key listed in the DID document; it does not sign` (INFO, at start) | One line per key in `PORTITOR_IDENTITY_RETIRED_KEY_FILES`; shows its key id |
| `STS token issued` (INFO) | Your connector got a token for the named audience |
| `STS client refused` (INFO) | A wrong client id or secret at the STS |
| `request refused` (INFO) | A caller's token was refused; `reason` says why (expired, replayed, wrong audience, untrusted issuer, ...). The caller only gets HTTP 401 |
| `presentation sent` (INFO) | A verifier got a presentation; `credentials` is how many it held |
| `issuance message refused` (INFO) | A `CredentialMessage` or offer was refused (HTTP 400); `reason` says why |
| `credential request sent` (INFO) | The issuer took a credential request (`holderPid`) |
| `credential request not sent` (INFO) | The issuer refused a request (HTTP 4xx), or it could not be signed; `reason` says why. Nothing was kept |
| `credential request outcome unknown; kept open` (WARN) | No answer in time, a network error, or HTTP 408, 429 or 5xx from the issuer; the request stays open for a late delivery |
| `repeated delivery acknowledged` (INFO) | The issuer sent an answer again that closed a request before; nothing was stored twice |
| `the issuer gave no usable Location for the request` (WARN) | The issuer took the request but gave no status URL on its own host; the delivery is still accepted, only the issuer's status cannot be shown |
| `identity store failed` (ERROR) | The database could not be read or written |
| `certificate chain reloaded` (INFO) | A renewed certificate chain for the same key is in use |
| `new certificate chain refused; the old one stays` (ERROR) | The chain file changed but cannot be used: unreadable, not PEM, or for another key; `reason` says which |
| `Gaia-X compliance credential stored` (INFO) | A run succeeded; `valid_until` is when the compliance credential expires |
| `Gaia-X compliance run failed; the stored credentials stay` (ERROR) | A run failed; `error` says why (the service refused, its answer did not check out, ...). It is tried again |

Used token ids (`jti`) are kept in the database until their token
expires, so a token used on one replica is refused on the others
([ADR 0029](adr/0029-several-replicas.md); before 1.3 they were kept in
memory). <!-- 1.3: check after integration -->

### Issuer logs

| Log message (level) | Meaning |
|---|---|
| `portitor-issuer listening` (INFO, at start) | Started; shows the DID, key id, issuer service URL and database |
| `retired key listed in the DID document; it does not sign` (INFO, at start) | One line per key in `PORTITOR_ISSUER_RETIRED_KEY_FILES`; shows its key id |
| `credential offer sent` (INFO) / `credential offer not taken` (WARN) | An offer (`POST /v1/offers`) reached the holder, or not (the call answers 502) |
| `credential request refused` (INFO) | A credential request was malformed and answered 400; `reason` says why |
| `request refused` (INFO) | A caller's token was refused (401), for example a DID that is not registered |
| `delivery given up`, `delivery refused by the holder, given up` | A credential delivery was stopped; the credentials stay issued ([Registering participants and issuing](DEPLOYING.md#registering-participants-and-issuing)) |
| `bundle key listed in the DID document` (INFO, at start) | `PORTITOR_ISSUER_BUNDLE_KEY_FILE` is set; shows the key id |
| `serving bundles at /bundles/` (INFO, at start) | `PORTITOR_ISSUER_BUNDLE_DIR` is set |
| `bundle file not readable`, `bundle manifest not readable` (WARN) | A request for a bundle file was answered 404 because the file or the version's manifest could not be read; check the folder |


## Traces and metrics

All three components can send traces and metrics with OpenTelemetry
([ADR 0023](adr/0023-observability.md)). They push over OTLP to an
OpenTelemetry Collector or any receiver that takes OTLP/HTTP (Prometheus
3 with `--web.enable-otlp-receiver`, Jaeger). There is no Prometheus
scrape endpoint.

**Switch it on.** Export is **off** unless an endpoint variable is set.
Set the receiver's base URL; the components add `/v1/traces` and
`/v1/metrics`:

```sh
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318
```

The start log then says `telemetry: exporting` (INFO), with `traces`
and `metrics` set to `true` or `false`. If the variable is already in
your environment (for example injected by the OpenTelemetry Operator),
export is on.

- Protocol: OTLP over HTTP with Protocol Buffers only (no gRPC, no
  JSON), without compression. `http://` and `https://` are both allowed:
  the receiver is your own. An `https://` receiver needs the CA bundle
  ([Private CAs](DEPLOYING.md#private-cas)).
- The SDK reads the `OTEL_*` values as written: a space around an
  endpoint makes it invalid. A setting the SDK refuses is logged
  (`telemetry: export is off`, ERROR) and the component runs on with
  logs only. A value the SDK does not know for compression or
  temporality only gets an SDK warning; the default is used.
- A failed export is logged by the SDK (target `opentelemetry_sdk`);
  its data is dropped, and the next batch is sent as usual.
- At shutdown, the last spans and metrics get about 3 s together. What
  is not sent by then is lost, logged as `telemetry: shutdown took too
  long; the last data is lost` (WARN; also `telemetry: the last data was
  not sent` or `telemetry: shutdown failed` when the receiver refused
  it).

**Variables.** Only OpenTelemetry's own variables; there is no
`PORTITOR_*` setting. Read by the Rust SDK 0.33 that Portitor uses:

| Variable | Use |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Switches export on for both signals; the receiver's base URL |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | A full URL for one signal, used instead of the base URL; each also switches export on, for its signal |
| `OTEL_EXPORTER_OTLP_HEADERS` (and `_TRACES_`, `_METRICS_`) | Headers for the receiver, `key=value,key2=value2`, for example an API key |
| `OTEL_EXPORTER_OTLP_TIMEOUT` (and `_TRACES_`, `_METRICS_`) | Milliseconds an export may take (default 10000; each request is also cut off after 10 s) |
| `OTEL_SERVICE_NAME` | `service.name`; default `portitor-connector`, `portitor-identity` or `portitor-issuer` |
| `OTEL_RESOURCE_ATTRIBUTES` | More resource attributes, for example `deployment.environment.name=prod`; `service.version` is Portitor's version unless set here |
| `OTEL_TRACES_SAMPLER`, `OTEL_TRACES_SAMPLER_ARG` | Which traces are kept (below); default every trace (`parentbased_always_on`) |
| `OTEL_BSP_SCHEDULE_DELAY`, `OTEL_BSP_MAX_QUEUE_SIZE`, `OTEL_BSP_MAX_EXPORT_BATCH_SIZE` | The batch of spans: milliseconds between exports (default 5000), spans kept waiting (2048; more are dropped), spans per export (512) |
| `OTEL_METRIC_EXPORT_INTERVAL` | Milliseconds between metric exports (default 60000) |
| `OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE` | `cumulative` (default), `delta` or `lowmemory`. Keep the default for Prometheus' OTLP receiver: it takes cumulative metrics, and delta only with an experimental feature flag ([Prometheus guide](https://prometheus.io/docs/guides/opentelemetry/)) |
| `OTEL_EXPORTER_OTLP_COMPRESSION` (and `_TRACES_`, `_METRICS_`) | Read, but compression is not built in: the SDK logs a warning and sends uncompressed |
| `OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT`, `OTEL_SPAN_EVENT_COUNT_LIMIT`, `OTEL_SPAN_LINK_COUNT_LIMIT` | Limits per span (default 128 each) |

No effect: `OTEL_SDK_DISABLED` (to switch export off, unset the
endpoint variables), `OTEL_EXPORTER_OTLP_PROTOCOL` (always
`http/protobuf`), `OTEL_BSP_EXPORT_TIMEOUT`,
`OTEL_BSP_MAX_CONCURRENT_EXPORTS` and `OTEL_METRIC_EXPORT_TIMEOUT` (only
the SDK's experimental async processors read them), and `OTEL_LOGS_*`
(logs stay on stdout).

**Cost and sampling.** In the 0.9 measurements the cost of tracing
every request was close to the noise between runs
([BENCHMARKS](BENCHMARKS.md#cost-of-traces-09)). Sampling does **not**
raise throughput: the spans are made in the process either way. It
lowers what is sent (about ten times fewer trace exports with one trace
in ten) and the load on your backend. To keep one trace in ten:

```sh
OTEL_TRACES_SAMPLER=parentbased_traceidratio
OTEL_TRACES_SAMPLER_ARG=0.1
```

Metrics are not sampled: they count every request, except the health
checks. With sampling, a log line can carry the trace id of a trace
that was not kept.

**What a trace shows.** One span per request served on the public and
the management port (`GET /dsp/2025-1/negotiations/{pid}`: the route
template, never the path with its ids), except the health checks, which
get no span; one per HTTP call sent (`POST`, with the host and port
called, never the URL); one per DSP outbox delivery
(`dsp.outbox.deliver`); and each negotiation and transfer state change
as a span event with both DSP process ids. Attributes are an allow-list:
method, route, status code, host and port, process ids and states;
never a token, an `Authorization` header, a key, a body or dataset
contents. The bridge from Rust's `tracing` also adds where in the code
the span was made (`code.*` attributes) and how long it was busy and
idle (`busy_ns`, `idle_ns`). Other log lines are not copied into
traces.

**Trace context stays inside the participant.** A `traceparent` header
(W3C Trace Context) is sent only on the connector's calls to its own
STS and to its own data sources (the dataset's `portitor:source`), and
read only on the identity component's `/sts/token`, and there only
after the client id and secret are checked. DSP and DCP calls to
other participants carry none, and one that arrives on the DSP port is
ignored: each side starts its own trace. To follow one negotiation
across two organisations, match the DSP process ids (`providerPid`,
`consumerPid`) in both sides' logs.

**Logs carry the trace id.** With export on, every JSON log line written
inside a span starts with `"trace_id":"<32 hex digits>"`, the
id of the trace it belongs to: search the logs for it. With export off,
log lines are as before.

**State changes in the log.** Each change is also one log line (INFO,
target `portitor::state`): `negotiation state changed` or `transfer
state changed`, with `portitor.dsp.process`, `portitor.dsp.role`,
`portitor.dsp.provider_pid`, `portitor.dsp.consumer_pid`,
`portitor.dsp.previous_state` (none for a new process) and
`portitor.dsp.state`.

**Audit line of each data transfer** (since 1.1,
[ADR 0027](adr/0027-push-transfers-and-object-storage.md) section 6,
[TD-31](RISKS_AND_TECHNICAL_DEBTS.md#td-31) first half). Each response
of the provider's data plane (`GET /data/{flowId}`), each push the
provider sends and each push the consumer receives
(`POST` or `PUT /push/{consumerPid}`) writes, once it has ended, one
line at INFO with the message `data transfer done` and the target
`portitor::audit`, with these fields:

| Field | Value |
|---|---|
| `flow` | The data flow id: the provider's process id of the transfer (`providerPid`; `portitor.dsp.provider_pid` in the state change lines), on both sides |
| `agreement` | The agreement id |
| `counterParty` | The consumer's DID |
| `direction` | `sent` (a pull served, a push sent) or `received` (a push received) |
| `bytes` | The bytes of data actually sent to the consumer |
| `outcome` | `complete` (all of it sent), `cut` (stopped before the end: the consumer closed the connection, or access was taken away while the data was flowing) or `failed` (the data source failed before or during the answer; `bytes` is then what was sent before) |

For a push the fields are the same, with these differences. A push sent
(as provider): `flow` is the provider's process id, `counterParty` the
consumer's DID, and `outcome` is `complete` when the consumer answered
2xx, `failed` when it answered anything else, did not answer or the
source failed, and `cut` when the transfer was suspended or terminated,
or the consumer's membership no longer held. A push received (as consumer): `direction`
is `received`, `flow` is still the provider's process id (so the two
partners' lines of one push can be matched; the object key is the
`consumerPid`), `counterParty` the provider's participant id, `bytes`
what arrived, and
`outcome` is `complete` when the object is stored, `cut` when the
sender's body broke off, and `failed` when the object storage failed.

For example:

```json
{"timestamp":"2026-11-24T10:00:00.000000Z","level":"INFO","fields":{"message":"data transfer done","flow":"urn:uuid:6f0c...","agreement":"urn:uuid:a1b2...","counterParty":"did:web:consumer.example","direction":"sent","bytes":18,"outcome":"complete"},"target":"portitor::audit"}
```

The message, the target and these fields are stable in 1.x
([STABILITY, Audit line](STABILITY.md#audit-line)).

A refused request (HTTP 401, on `/data` or `/push`) writes no audit
line. As provider, `agreement` and `counterParty` are the ones the data
flow keeps from its start message (from 1.2). A flow that started
before 1.2 and has no transfer in the database has them empty, as in
1.1. The line never holds
the token or the data source's URL. Portitor keeps no audit store: keep
these lines as long as your rules require (ADR 0025, "Audit"). With
`RUST_LOG` set, keep `portitor::audit=info` in it.

**Metrics.** Names follow OpenTelemetry's
[naming rules](https://opentelemetry.io/docs/specs/semconv/general/naming/);
the two HTTP histograms are the
[HTTP semantic conventions'](https://opentelemetry.io/docs/specs/semconv/http/http-metrics/),
with their advised buckets (5 ms to 10 s). A Prometheus receiver turns
dots into underscores and adds the unit, for example
`http_server_request_duration_seconds` and
`portitor_negotiation_ended_total`.

| Name | Instrument, unit | Attributes | Component |
|---|---|---|---|
| `http.server.request.duration` | histogram, `s` | `http.request.method`, `http.route` (when a route matched), `http.response.status_code`, `url.scheme` (`http`: TLS ends before the component), `network.protocol.version`, `error.type` (5xx). Health checks are not measured | all |
| `http.client.request.duration` | histogram, `s` | `http.request.method`, `server.address` and `server.port` (only for calls inside the participant: the own STS and data sources), `http.response.status_code` (when answered), `error.type` (4xx, 5xx, `timeout` or `_OTHER` when unanswered, or when the body of a 2xx answer cannot be read) | all |
| `portitor.negotiation.ended` | counter, `{negotiation}` | `portitor.dsp.role` (`provider`, `consumer`), `portitor.dsp.state` (`FINALIZED`, `TERMINATED`) | connector |
| `portitor.transfer.ended` | counter, `{transfer}` | `portitor.dsp.role`, `portitor.dsp.state` (`COMPLETED`, `TERMINATED`) | connector |
| `portitor.dsp.request.refused` | counter, `{request}` | `error.type`: `unauthenticated` (HTTP 404, or 401 on the catalog routes), `invalid_message` or `message_rules` (the bundle's message shapes; both HTTP 400) | connector |
| `portitor.outbox.message.resent` | counter, `{message}` | `portitor.outbox.name` (`dsp`, `credential_delivery`), and for `dsp` `portitor.dsp.process` (`negotiation`, `transfer`) and `portitor.dsp.role` | connector, issuer |
| `portitor.outbox.message.given_up` | counter, `{message}` | as above | connector, issuer |
| `portitor.harvest.run.ended` | counter, `{run}` | `portitor.harvest.kind` (`datasets`, `bundle`), `portitor.harvest.outcome` (`succeeded`, `partial`, `unchanged`, `failed`) | connector |
| `portitor.harvest.dataset.rejected` | counter, `{dataset}` | `portitor.harvest.check` (the check of the job log) | connector |
| `portitor.http.request.panicked` | counter, `{request}` | `http.route` (when a route matched). A request handler panicked (a bug); the request got HTTP 500 | all |
| `portitor.task.panicked` | counter, `{task}` | `portitor.task.name`: `outbox_delivery`, `dataset_harvest`, `bundle_harvest`, `identity_check` (connector), `certificate_watch`, `gaiax_schedule`, `gaiax_run` (identity), `issuer_outbox`, `credential_decision` (issuer) | all |
| `portitor.bundle.in_use` | gauge, `{bundle}` | `portitor.bundle.version`: 1 for the version in force, 0 for one replaced since the start | connector, with a bundle |
| `portitor.bundle.file.skipped` | gauge, `{file}` | none. Files of the bundle version in force that are skipped because this connector does not know their role; what they hold is not enforced. 0 when every file is used | connector, with a bundle |

A resend is a retry of a message, a message saved before a restart
and sent again, or a DSP message sent again after its sending
panicked; a give-up is a message the outbox stopped trying (the
negotiation or transfer is then terminated or not started, as the log
says; for the issuer, the delivery is `FAILED`). Every service also
reports its `service.name`, `service.version` and the SDK's resource
attributes.

## Backup and restore

Keep the issuer's key backup separate from its database backup.

**PostgreSQL.** Back up each component's database with the tools of
PostgreSQL, for example `pg_dump --format=custom` while the components
run, and restore with `pg_restore` into an empty database while they
are stopped. Which tool fits your server (dumps, base backups,
point-in-time recovery) is in the PostgreSQL manual, chapter "Backup
and Restore" (<https://www.postgresql.org/docs/current/backup.html>).
Replicas of a component share one database, so there is one backup per
component, not one per container.

**SQLite (deprecated).** The connector's database is
`/data/portitor.db` plus, while it runs, `portitor.db-wal` and
`portitor.db-shm`. The identity
(`/data/portitor-identity.db*`) and the issuer
(`/data/portitor-issuer.db*`) work the same way.

- **Simple and safe:** stop the container, copy all `portitor.db*` files,
  start it again.
- **Online:** from a separate container that has the `sqlite3` tool and
  mounts the same volume, run `sqlite3 /data/portitor.db ".backup '/backup/portitor-$(date +%F).db'"`.
  Do not copy only `portitor.db` while the connector runs.
- **Restore:** stop the container, replace the `portitor.db*` files with the
  backup (delete any old `-wal` and `-shm` files), start it.

Negotiations and transfers that were in progress at backup time continue
from their saved state.

Data received by push (since 1.1) is in your object storage, not in
`/data`: back the push bucket up with your storage's own tools.

## Rotating a key

The identity and the issuer each sign with one P-256 key. A new key
gets a new key id (its thumbprint). To change it without a gap, list
the old **public** key as retired for a while: what the old key signed
keeps verifying, and only the new key signs. EDC IdentityHub rotates
the same way: the old key stays listed until it is revoked
([EDC decision record 2024-09-02](https://github.com/eclipse-edc/IdentityHub/blob/main/docs/developer/decision-records/2024-09-02-resource-operations/README.md)).

1. Make the new key, as at the first deployment:
   `openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out new-key.pem`.
2. Write the old public key to a file:
   `openssl pkey -in old-key.pem -pubout -out old-key.pub.pem`.
3. Mount `new-key.pem` as the key file and `old-key.pub.pem` read-only,
   and set `PORTITOR_IDENTITY_RETIRED_KEY_FILES` (or
   `PORTITOR_ISSUER_RETIRED_KEY_FILES`) to its path. With Gaia-X, also
   mount a certificate chain for the new key: the identity does not
   start with a chain for another key. Restart. The start log shows the
   new key id and one `retired key listed` line. The start fails when a
   retired file holds a private key, a key that is not P-256, the
   current key or, for the issuer, the bundle key, and when a key is
   given twice.
4. Destroy the old private key at once, with its backups: a retired
   key never signs.
5. Wait until everything the old key signed has expired:
   - identity: tokens and presentations (5 minutes), access tokens
     given to issuers with a credential request (30 days, or until the
     request is delivered). With Gaia-X, start a run at once
     (`POST /v1/gaiax/compliance`): Gaia-X services want the chain of
     the signing key, and the chain is published for the current key
     only. When the run succeeds, the Gaia-X credentials of the old key
     are replaced;
   - issuer: every credential it issued (their `validityDays`). Status
     lists are signed again with the new key at start.
     <!-- 1.3: check after integration: from 1.3 the signed lists are stored in the database (ADR 0029, D5); say when a list signed by the old key is signed again, before step 6 removes that key -->
     To end sooner,
     reissue the credentials with the new key.
6. Remove the file from the setting and restart. The old key id leaves
   the DID document; what the old key signed no longer verifies.

Verifiers cache DID documents (Portitor for
`PORTITOR_DID_CACHE_SECONDS`, 5 minutes by default; see
[DID document cache](DEPLOYING.md#did-document-cache)), so a new key id may be
refused for that long after step 3 by a verifier that has not seen the
new document. Do step 3 when the component is quiet. For the same
reason a key removed in step 6 is still accepted for that long.

### Rotating a key with several replicas

With several containers of the identity or the issuer, a partner may
read the DID document from any of them. Restarting them one by one with
the steps above would let some show the old key only while others
already sign with the new one. Do it in two rolling steps instead, so
every replica lists both keys before any signs with the new one
([ADR 0029](adr/0029-several-replicas.md), D7;
[TD-58](RISKS_AND_TECHNICAL_DEBTS.md#td-58)):

1. Make the new key (step 1 above) and write **its public key** to a
   file: `openssl pkey -in new-key.pem -pubout -out new-key.pub.pem`.
2. On every replica, keep the old key as the key file and list
   `new-key.pub.pem` in `PORTITOR_IDENTITY_RETIRED_KEY_FILES` (or
   `PORTITOR_ISSUER_RETIRED_KEY_FILES`). Restart the replicas one by
   one. The new key is now listed on every replica and signs nothing.
   The start checks allow this: the file holds a public key that is not
   the current one.
3. Wait `PORTITOR_DID_CACHE_SECONDS` of your partners (5 minutes by
   default), so their caches hold the document with both keys.
4. On every replica, mount `new-key.pem` as the key file and list the
   old public key (`old-key.pub.pem`, step 2 above) as retired, instead
   of the new one. With Gaia-X, mount the chain of the new key too.
   Restart the replicas one by one. While some replicas still run the
   old key, every replica lists both, and each accepts the access
   tokens the others signed.
5. Go on with steps 4 to 6 above: destroy the old private key, wait,
   then remove the retired file and restart the replicas one by one.

**A leaked or lost key.** Never list a leaked key as retired: anyone
with it could sign in your name while it is listed. Change the key
without a retired list; what the old key signed stops verifying, and
the issuer must reissue every credential. A lost issuer key (not
leaked) can be listed as retired, since only its public key is needed.

## Upgrade and rollback

1. Read the [CHANGELOG](../CHANGELOG.md) entries between your version and
   the new one.
2. Back up the database ([Backup and restore](#backup-and-restore)).
3. Replace the containers with the new image. Database migrations run at
   start, forward only.
4. Check the start log and the readiness check on the management port
   (`GET /api/check/readiness` answers HTTP 200;
   [Health and logs](#health-and-logs)). `/health` only shows that the
   process runs.

With several replicas on PostgreSQL, the first replica of the new
release that starts runs the migrations, under a lock in the database;
replicas that start at the same time wait for it, then find nothing to
do. Migrations run forward only, so a replica of the older release
that starts after them refuses the database. Unless the notes of the
release say that its replicas may run next to older ones, stop every
replica of the older release, then start the new ones.
<!-- 1.3: check after integration -->

Rollback: an older version refuses a database that a newer version
migrated (`database schema version N is newer than this Portitor`). Restore
the backup from step 2, then start the older image. What was written
after the backup is lost. This stays so in 1.x
([TD-22](RISKS_AND_TECHNICAL_DEBTS.md#td-22), [STABILITY](STABILITY.md#database)).

Messages the connector was sending when it stopped are saved and sent
again (ADR 0007): from 1.3 by any running replica once their lease
ends, or at the next start, so an upgrade does not leave negotiations
waiting. <!-- 1.3: check after integration --> A partner may receive such a message twice; that is handled.

### From 1.2 to 1.3

1.3 adds PostgreSQL and lets several containers of each component share
one database ([ADR 0029](adr/0029-several-replicas.md)).
<!-- 1.3: check after integration -->

1. **Back up first.** Migrations run at start, forward only. To go back
   to 1.2, restore the backup.
2. **SQLite is deprecated.** A component on a SQLite file keeps
   working, for one process, and logs at start `SQLite is deprecated
   since 1.3 and is removed in 2.0`. A second process on the same file
   now refuses to start. Nothing else to do until 2.0.
3. **PostgreSQL is optional in 1.3** for an existing deployment. To use
   it, read [From SQLite to PostgreSQL](#from-sqlite-to-postgresql)
   first: there is no tool yet that copies your data.
4. **Several replicas** need PostgreSQL
   ([DEPLOYING, Database](DEPLOYING.md#database)), and the two-step
   [key rotation](#rotating-a-key-with-several-replicas).

### From SQLite to PostgreSQL

The tool that copies a SQLite file into PostgreSQL comes in a later
1.x release, before 2.0 removes SQLite
([ROADMAP](ROADMAP.md#later-not-planned-for-a-release-yet)). Until
then, a component moved to PostgreSQL starts with an empty database:

- the connector forgets its negotiations, agreements, transfers and
  data flows, while partners keep theirs. Existing agreements can no
  longer be used, and running transfers stop;
- the identity forgets its credentials: ask the issuers again
  ([Getting a credential](DEPLOYING.md#getting-a-credential));
- the issuer forgets its participants, the credentials it issued and
  their status lists, so it can no longer revoke them.

So move a component only if it can start over, for example a new
dataspace or a test. Otherwise keep it on its SQLite file, with one
container, until the copy tool exists. For a move: set
`PORTITOR_DATABASE_URL` and `PORTITOR_DATABASE_PASSWORD_FILE`, and
leave `PORTITOR_DB_FILE` unset (a component with both refuses to
start).
<!-- 1.3: check after integration -->

### From 1.1 to 1.2

1.2 moves the built-in data plane into its own crate,
`portitor-dataplane`. It still runs inside the connector, in the same
image ([ADR 0028](adr/0028-separate-pluggable-data-plane.md), step 1).
No setting or route was added, removed or renamed. Nothing has to
change in your configuration.

1. **Back up `/data` first.** Migrations 14, 15, 16 and 17 run at
   start, forward only. They add the columns `data_flow.source`,
   `data_flow.agreement_id`, `data_flow.counter_party_id`,
   `transfer.flow_step`, and the lease of a transfer
   (`transfer.state_before_step`, `transfer.lease_holder`,
   `transfer.lease_until`, and the partial index
   `transfer_recorded_step`). Migration 14 gives each existing data flow the
   `portitor:source` its dataset has in the harvest's read copy.
   Migration 15 gives it the agreement and the consumer of its
   transfer. To go back to 1.1, restore the backup.
2. **Behaviour change, provider: a transfer keeps the source it started
   with.** The connector now sends the source to the data plane in the
   start message, and the data flow keeps it until it ends. A
   `portitor:source` you change in the datasets file reaches only
   transfers that start after the next harvest run. A running transfer,
   also one suspended and started again, still reads the old source. To
   move it to the new source, end it
   (`POST /v1/provided-transfers/{providerPid}/termination`); the
   consumer asks for a new transfer.
3. **Transfers started under 1.1 whose dataset had no `portitor:source`
   at the upgrade** keep no source. 1.1 refused them too, and they stay
   refused: a `/data` request gets 401, and a push transfer is
   terminated at start (`push: cannot start; the transfer is
   terminated`). Adding the source later does not help these
   transfers. End a pull transfer of this kind with the termination
   route above, so the consumer is told; it asks for a new transfer.
4. **Fixed, provider: the data plane is told once, after the transfer
   is saved**
   ([TD-48](RISKS_AND_TECHNICAL_DEBTS.md#td-48)). Nothing to do. While
   its data plane step runs, a transfer is leased in the database for
   at most 30 seconds. **New answer:** a partner's transfer message (or
   your `POST /v1/provided-transfers/{providerPid}/termination`) that
   comes in that moment gets HTTP 503 with `Retry-After: 1` and reason
   `the transfer is being changed; retry`; nothing was written, and the
   partner retries (Eclipse EDC does). In 1.1 it waited instead. A step
   left over by a process that stopped runs within about 40 seconds
   (sweep every 10 seconds). New log lines: `data plane step left over
   ran; its lease had ended` (INFO), `transfer not taken up: ...`
   (ERROR, at start or in the sweep) and `transfer sweep: the transfers
   cannot be read` (ERROR); see the table above. `portitor-connector
   listening` has a new field, `process`: the random id this process
   leases transfers with.
5. **Logs.** `audit: transfer store failed` is gone: the audit line now
   takes the agreement and the consumer from the data flow. The audit
   line itself (`data transfer done`, target `portitor::audit`) does
   not change. The other lines of the data plane keep their text, but
   their `target` field now starts with `portitor_dataplane::` instead
   of `portitor_connector::`. If your `RUST_LOG` or your log shipping
   selects lines by target, add `portitor_dataplane`.

### From 1.0 to 1.1

1.1 adds push transfers, object storage and ways to end access
([ADR 0027](adr/0027-push-transfers-and-object-storage.md)). No setting
or route was removed or renamed; the new ones are listed in
[STABILITY](STABILITY.md). Nothing has to change in your configuration.

1. **Back up `/data` first.** Migrations 12 and 13 run at start,
   forward only: the table `flow_membership`, the columns
   `transfer.data_address` and `data_flow.push_address`, and the table
   `push_receipt`. To go back to 1.0, restore the backup.
2. **Behaviour change, provider: the consumer's membership is checked
   again on every `/data` request and while a download runs.** A
   consumer whose `MembershipCredential` was revoked, has expired or
   whose issuer left `PORTITOR_TRUSTED_ISSUERS` gets 401, and the
   transfer is terminated. Transfers started before the upgrade have no
   membership kept: they are served as in 1.0 until their token expires,
   with one INFO line per flow. They never get a new token: a provider
   policy that starts such a suspended transfer again (only a scripted
   one, as in the TCK build) terminates it instead, and the consumer
   asks for a new transfer, which is checked
   ([DEPLOYING, Revocation and running transfers](DEPLOYING.md#revocation-and-running-transfers)).
   Each check reads the status list through the cache
   (`PORTITOR_STATUS_LIST_CACHE_SECONDS`); with a cache of `0`, every
   `/data` request fetches the list.
3. **Behaviour change, provider: suspend and terminate cut running
   downloads.** In 1.0 a running download went on to its end.
4. **Behaviour change, both sides: a partner whose membership was
   revoked can still end a transfer it shares with you** (TD-47); every
   other message from it is refused as before.
5. **Logs.** Each pull, push sent and push received writes the audit
   line `data transfer done` (target `portitor::audit`,
   [Traces and metrics](#traces-and-metrics)). With `RUST_LOG` set, keep
   `portitor::audit=info` in it. The other new lines are in
   [Health and logs](#health-and-logs), marked 1.1.
6. **Only if you use push or `s3://` sources:** set the object storage
   settings and create the push bucket (with a lifecycle rule that
   aborts incomplete multipart uploads), let your reverse proxy pass
   large, long requests on `/push/`, and keep the egress firewall: a
   push goes to an address the consumer chose
   ([DEPLOYING, Object storage and push transfers](DEPLOYING.md#object-storage-and-push-transfers)).
   `AWS_*` variables already in your environment are checked only when
   `PORTITOR_PUSH_BUCKET` is set; otherwise unusable ones are ignored.
   As consumer, the push token lives `PORTITOR_DATA_TOKEN_SECONDS`
   from your transfer request, not from the provider's start.

### From 0.9 to 1.0 in one step

For an operator who runs 0.9 and goes straight to 1.0. This is the
short list of what to do; the notes of each release below give the
details and the log lines. 1.0 is the 0.14 release candidate with no
new features ([ROADMAP, 1.0](ROADMAP.md#10-stable)), so 1.0 adds nothing
to this list unless its [CHANGELOG](../CHANGELOG.md) entry says so.
0.10 has no step.

**Database.** Back up the connector's database (`/data/portitor.db*`):
0.12 migrates it to schema version 11, and 0.9 cannot open it
afterwards. The identity's and the issuer's databases do not change
between 0.9 and 1.0.

**Before you start the new images, change the configuration.** A
service that finds an old or wrong value does not start, and its log
names the setting.

1. Rename three settings
   ([table](#from-012-to-013), point 1): `PORTITOR_ALLOW_HTTP_CALLBACKS`
   to `PORTITOR_ALLOW_HTTP` (all three),
   `PORTITOR_IDENTITY_CERTIFICATE_CHAIN` to
   `PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE` (identity) and
   `PORTITOR_ISSUER_CREDENTIALS_FILE` to
   `PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE` (issuer).
2. Connector: set `PORTITOR_PARTICIPANT_ID` (a `did:web` DID) and
   `PORTITOR_PUBLIC_URL` (an `http://` or `https://` URL without a query
   or fragment). Both are required and checked at start
   ([0.13](#from-012-to-013), points 3 and 7).
3. Issuer: add `credentialSchema` to every entry of the
   [credential definitions file](DEPLOYING.md#credential-definitions-file)
   ([0.12](#from-011-to-012), point 1).
4. All three, the form of a value
   ([0.13](#from-012-to-013), points 4, 8 and 9;
   [0.14](#from-013-to-014), point 1): an empty value means "not set"
   (check the table of what changed for a setting that is set but
   empty); a switch is `true` or `false`; a list setting that is set
   must name something; an item of a list is at most 4096 bytes.
5. Connector, datasets file: policies are read strictly. A field the
   connector does not know, a `duty` inside a permission, `assigner` or
   `assignee` in an offer, or a JSON key given twice keeps the dataset
   out of the catalog; the harvest job names it
   ([0.12](#from-011-to-012), point 2;
   [Policies](DEPLOYING.md#policies)). Check your datasets before the
   upgrade.

**Change your management API clients.**

6. `POST /v1/negotiations` takes `counterPartyAddress` and
   `counterPartyId` in place of `providerAddress` and `providerId`, and
   the negotiation and transfer answers return them under the new names
   ([0.13](#from-012-to-013), point 2).
7. An empty `datasetId` (connector) or `holderPid` (identity) answers
   400; leave `holderPid` out for a new id ([0.13](#from-012-to-013),
   point 13).
8. `POST /v1/catalog/request` answers 502 when the provider's catalog
   has no `participantId` or one that differs from `counterPartyId`
   ([0.14](#from-013-to-014), point 2).

**Check your alerts and monitoring.**

9. A DSP caller that cannot be authenticated gets 404, not 401, on the
   negotiation and transfer routes; the catalog still answers 401
   ([0.12](#from-011-to-012), point 3).
10. The connector's DCP check answers 503, not 401, when a server it
    needs (a DID document, a Credential Service, a status list, the STS)
    did not answer, or answered 408, 429 or 5xx, or took longer than
    20 s. A partner's outbox then tries again. 0.11, CHANGELOG.
11. A panic no longer stops the process: watch
    `portitor.http.request.panicked` and `portitor.task.panicked`
    ([0.12](#from-011-to-012), point 5). The metric
    `portitor.outbox.message.given_up` counts fewer messages
    ([0.14](#from-013-to-014)).

**To know, nothing to do.**

- Status lists are cached by the connector (0.11): a revoked credential
  can be accepted for up to `PORTITOR_STATUS_LIST_CACHE_SECONDS`
  (default 300) after the revocation. 0.9 fetched the list at every
  check; set `0` to keep that
  ([Status list cache](DEPLOYING.md#status-list-cache)).
- Request size limits (0.11): `PORTITOR_MAX_REQUEST_BYTES` and
  `PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES` default to 2 MiB, as before.
  The small DSP messages, `POST /sts/token` and the issuer's credential
  request route have a fixed limit of 64 KiB. A larger body gets 413.
- The connector's outbox stops trying a DSP message at once when the
  answer is a refusal (400, 404, 409, ...); it tries again only when no
  answer came or the answer was 408, 429 or 5xx (0.11, ADR 0007).
- New optional settings with defaults that keep the 0.9 behaviour:
  `PORTITOR_DID_CACHE_SECONDS` (0.12) and the retired key files of
  [Rotating a key](#rotating-a-key) (0.11).
- A credential is refused when the key that signed it belongs to
  another DID in its issuer's DID document
  ([0.12](#from-011-to-012), point 4).
- The connector's management port answers 503 to a request that takes
  longer than 60 s (0.13, CHANGELOG).
- The release binaries are about 13% larger (0.12).

**Rollback to 0.9.** Restore the connector's database from the backup,
put back the three old setting names and the old field names in your
clients, and remove `credentialSchema` from the definitions file: a 0.9
issuer refuses a field it does not know.

### Upgrades from 0.4 to 0.9, in short

What you still have to do, or to know, when an upgrade crosses one of
these releases. "Back up" means: that component's database is migrated
at the first start of the new version, and the older version refuses
the migrated file, so a rollback needs the backup. A release that is not
listed has no step here; read its [CHANGELOG](../CHANGELOG.md) entry.

**0.4 to 0.5: every DSP request is authenticated with DCP (ADR 0016).**

- Before: deploy the identity with a credential
  ([step 2](DEPLOYING.md#step-2-identity-every-participant)) and set
  `PORTITOR_STS_URL`, `PORTITOR_STS_CLIENT_SECRET_FILE` and
  `PORTITOR_TRUSTED_ISSUERS` on the connector. Without them it does not
  start.
- Upgrade at the same time as your partners: a 0.4 connector cannot
  talk to a 0.5 one.
- Before: finish or terminate the negotiations and transfers in
  progress. 0.5 cannot match them to the participant that started them:
  a provider answers that they do not exist, and a consumer refuses the
  provider's callbacks.
- After: negotiate again the agreements 0.4 made. They name
  `urn:portitor:unauthenticated` as the consumer (`assignee`), and no
  transfer starts under them.
- No database migration; a rollback to 0.4 needs no backup.

**0.5 to 0.6: the issuer.**

- Back up the identity's database (`/data/portitor-identity.db*`). The
  connector's is unchanged.
- The test-only `test-issuer` tool is gone: get credentials from your
  dataspace's issuer ([Getting a credential](DEPLOYING.md#getting-a-credential)).
  Credentials issued before stay valid.

**0.6 to 0.7: dataset harvest (ADR 0020) and bundle (ADR 0005).**

- Back up the connector's database (`/data/portitor.db*`, schema
  version 8). The identity's and the issuer's are unchanged.
- A datasets file that cannot be read, or a dataset that breaks a rule,
  no longer stops the start: the dataset is rejected and logged. This
  includes an offer with a credential constraint on a type not in
  `PORTITOR_POLICY_CREDENTIALS`, and a `portitor:source` that is not an
  HTTP(S) URL. To keep a strict start, check `GET /v1/harvest-jobs` in a
  start-up probe (latest `outcome` is `succeeded`). `PORTITOR_DATASETS`
  (removed in 0.2) still stops the start.
- Datasets you remove from the file are withdrawn at the next run, not
  at the next restart. `PORTITOR_HARVEST_INTERVAL_SECONDS=0` keeps the
  0.6 rhythm (read only at start).
- A distribution keeps its own metadata in the catalog (`@id`,
  `dcat:accessURL`, other properties).
- Keep `/data` on a volume: the verified bundle is kept in
  `/data/bundle-cache`.

**0.7 to 0.8: Gaia-X (ADR 0021, ADR 0022), optional.**

- Back up the identity's database (schema version 3; migrated with or
  without Gaia-X on). After a rollback, credentials received after the
  backup are lost; ask the issuer again. The connector and the issuer
  can go back to 0.7 as they are.
- The DID documents of the identity and the issuer give their key
  `alg: ES256` (RFC 7517 section 4.4). DCP ignores it; nothing to do.
- In the presentation API a scope type `T` also matches a credential of
  type `gx:T` whose `@context` names a Gaia-X context.
- Set `PORTITOR_GAIAX_COMPLIANCE_ISSUERS` on the provider before you
  publish an offer that requires Gaia-X compliance.

**0.8 to 0.9: health checks, traces and metrics (ADR 0023).**

- No database changes. Before a rollback to 0.8, move your probes back
  to `/health`: 0.8 has no `/api/check` routes.
- The management port is always bound, also without
  `PORTITOR_MANAGEMENT_API_KEY_FILE`. On a shared host or pod where
  that port is taken, set `PORTITOR_MANAGEMENT_BIND` to a free one, or
  the component does not start.
- Move Kubernetes probes from `GET /health` to the checks on the
  management port ([Health and logs](#health-and-logs)).
- The images check their own readiness (`HEALTHCHECK`). Docker Swarm
  replaces an unhealthy container; plain `docker run` only shows it. A
  connector whose own identity or STS is down becomes unhealthy after
  about 20 s.
- Export turns on if `OTEL_EXPORTER_OTLP_ENDPOINT` (or the `_TRACES_`
  or `_METRICS_` one) is already in the environment, for example set by
  the OpenTelemetry Operator. Unset it if you do not want export.
- Log changes: each negotiation and transfer state change is one INFO
  line (target `portitor::state`); a failed readiness check is logged
  when its result changes; the start line `management API listening` is
  now `management port listening`.
- `portitor-connector` and `portitor-identity` refuse any argument
  other than `health` (exit code 2). An empty
  `PORTITOR_MANAGEMENT_API_KEY_FILE` counts as unset in all three.

### From 0.11 to 0.12

Back up the connector's database before the upgrade: 0.12 adds three
indexes to it (schema version 11). The identity's and the issuer's
files do not change.

1. **Issuer, breaking: `credentialSchema` in the definitions file.**
   Before you upgrade the issuer, add `"credentialSchema": "<URL>"` to
   every entry of the
   [credential definitions file](DEPLOYING.md#credential-definitions-file). A 0.12
   issuer does not start with an entry that lacks it; the line says
   which (`"<id>": credentialSchema is required`). If your dataspace has
   no schema for a credential yet, agree on the URL where it will be
   published and use that. Credentials issued before stay valid, and
   participants change nothing.
2. **Connector: policies are read strictly.** At the first start of
   0.12 every stored dataset is checked again. A dataset whose offer
   has a field the connector does not know (a `duty` inside a
   permission, a `unit` on a constraint, a misspelled key, `assigner` or
   `assignee`) leaves the catalog until its source is corrected; the
   harvest job says which field, under the check `odrl`. Check your
   datasets before the upgrade ([Policies](DEPLOYING.md#policies)). Agreements made
   before stay valid.
3. **Connector: 404 for a refused caller.** A caller that cannot be
   authenticated gets HTTP 404, not 401, on the negotiation and
   transfer routes; the catalog still answers 401, and 503 is
   unchanged. If you monitor 401 answers, also watch the log line
   `DSP request refused: caller not authenticated` and its `status`
   field.
4. **Verifier and identity:** a credential is refused when the key that
   signed it is listed in its issuer's DID document under the id of
   another DID (DCP v1.0.1, 5.4.3, step 4; ADR 0016). Credentials from
   `portitor-issuer` and from EDC IdentityHub are not affected.
5. **All three: a panic no longer stops the process.** Watch the
   metrics `portitor.http.request.panicked` and
   `portitor.task.panicked`: each count is a bug to report. Keep the
   restart policy. The binaries are about 13% larger.
6. Nothing to do: `PORTITOR_DID_CACHE_SECONDS` is new and optional, and
   its default, 300 seconds, is what every version before did. A
   transfer start no longer slows down as negotiations pile up, and
   small answers no longer wait 43 ms ([BENCHMARKS](BENCHMARKS.md)).

### From 0.12 to 0.13

0.13 freezes the interface for 1.0 ([STABILITY](STABILITY.md)). Several
things changed before the freeze, because after 1.0 they could only
change in 2.0. The databases need no manual step. Change your
configuration and your management API clients **before** you start the
new images.

1. **Breaking, all three: three settings have new names.** A service
   that finds an old name set (to a value that is not empty) does not
   start; the log line `... stopped` names the new one, for example
   `PORTITOR_ALLOW_HTTP_CALLBACKS was renamed in 0.13; set
   PORTITOR_ALLOW_HTTP instead`. The meanings did not change.

   | Service | Old name | New name |
   |---|---|---|
   | connector, identity, issuer | `PORTITOR_ALLOW_HTTP_CALLBACKS` | `PORTITOR_ALLOW_HTTP` |
   | identity | `PORTITOR_IDENTITY_CERTIFICATE_CHAIN` | `PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE` |
   | issuer | `PORTITOR_ISSUER_CREDENTIALS_FILE` | `PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE` |

   `PORTITOR_STS_ALLOW_HTTP` is another setting and keeps its name.
2. **Breaking, connector management API: `counterPartyAddress` and
   `counterPartyId`.** `POST /v1/negotiations` takes them in place of
   `providerAddress` and `providerId`, and the answers of
   `/v1/negotiations` and `/v1/transfers` (start, one, list) return
   them under the new names. They are the names of Eclipse EDC and of
   the catalog calls. A request with an old name gets HTTP 400
   (`unknown field`). Change every client that starts a negotiation or
   reads these two fields. `providerPid` and `consumerPid` did not
   change.
3. **Breaking, connector: `PORTITOR_PARTICIPANT_ID` and
   `PORTITOR_PUBLIC_URL` are required.** Before, a connector without
   them started as `did:web:localhost%3A8080` at
   `http://localhost:8080`. Now it does not start
   (`PORTITOR_PARTICIPANT_ID: required ...`). A production connector
   set both already; check test and development setups.
4. **Behaviour change, all three: an empty value means "not set".**
   Spaces around a value are dropped, and a setting that is empty (or
   only spaces) counts as not set: a required one stops the start, an
   optional one takes its default. The identity and the issuer mostly
   did this already; the connector did not. What is different for a
   setting that is **set but empty**:

   | Setting | Before | From 0.13 |
   |---|---|---|
   | connector: `PORTITOR_PARTICIPANT_ID`, `PORTITOR_PUBLIC_URL` | an empty id or URL was used | the start stops (required) |
   | connector: `PORTITOR_BIND`, `PORTITOR_DB_FILE`, `PORTITOR_DATASETS_FILE` | an empty address or path was used (and failed, or gave an empty catalog) | the default (`0.0.0.0:8080`, `portitor.db`, no datasets file) |
   | connector: `PORTITOR_HARVEST_INTERVAL_SECONDS`, `PORTITOR_DATA_TOKEN_SECONDS` | the start stopped (invalid value) | the default |
   | connector: `PORTITOR_DCP_SCOPES` | the start stopped (`set but empty`) | the default scope |
   | connector: `PORTITOR_STS_CLIENT_SECRET_FILE` | the start stopped (cannot read the file) | the start stops (required) |
   | identity, issuer: `PORTITOR_BIND`, `PORTITOR_DB_FILE` | an empty address or path was used (and failed) | the default |
   | identity: `PORTITOR_SCOPE_TYPE_ALIASES` | no alias besides DCP's own | the default alias, `org.eclipse.edc.vc.type` |
   | issuer: `PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER` | the start stopped (invalid value) | the default, 10 |

   If you set `PORTITOR_SCOPE_TYPE_ALIASES` to an empty value to switch
   EDC's alias off, set it to `org.eclipse.dspace.dcp.vc.type` (DCP's
   own alias) instead. The content of a secret file is read as before.
   The connector also trims values now: a value with a space or a line
   end around it, which failed before, works.
5. **Bundle authorities, nothing to do:** a file whose role a connector
   does not know may now have any media type, not only the three
   ([Schema and vocabulary bundle](DEPLOYING.md#schema-and-vocabulary-bundle)).
6. New in 0.13, nothing to do: the catalog calls of the management API
   ([Management API](DEPLOYING.md#management-api)).
7. **Behaviour change, connector: `PORTITOR_PARTICIPANT_ID` and
   `PORTITOR_PUBLIC_URL` are checked at start**, as the identity's and
   the issuer's already were. The id must be a `did:web` DID; the URL an
   `http://` or `https://` URL without a query or fragment. A wrong
   value stops the start and the log names the setting. Before, such a
   connector started and its partners refused its messages.
8. **Behaviour change, all three: a switch is `true` or `false`.**
   `PORTITOR_ALLOW_HTTP` and `PORTITOR_STS_ALLOW_HTTP` with any other
   value (`1`, `yes`, `on`) stop the start; before, such a value was
   read as off without a word. Upper or lower case does not matter, and
   empty still means not set (off).
9. **Behaviour change, all three: a list that is set must name
   something.** A comma-separated setting whose value holds only commas
   and spaces (`","`) stops the start: `PORTITOR_TRUSTED_ISSUERS`,
   `PORTITOR_DCP_SCOPES`, `PORTITOR_POLICY_CREDENTIALS`,
   `PORTITOR_GAIAX_COMPLIANCE_ISSUERS`, `PORTITOR_SCOPE_TYPE_ALIASES`,
   `PORTITOR_IDENTITY_RETIRED_KEY_FILES`,
   `PORTITOR_ISSUER_RETIRED_KEY_FILES`. Before, some of them read such a
   value as an empty list and some stopped. Leave a list unset (or
   empty) for its default. To switch EDC's scope alias off, see point 4.
10. **Behaviour change, connector, nothing to do:** a bundle file whose
    role the connector does not know is skipped; before, the whole
    bundle was refused. The connector does not enforce what such a file
    holds. Watch the new gauge `portitor.bundle.file.skipped` and the
    WARN line `bundle file skipped: ...`
    ([Schema and vocabulary bundle](DEPLOYING.md#schema-and-vocabulary-bundle)).
11. **Behaviour change, connector as provider, nothing to do:** a
    transfer request from the agreement's consumer that arrives while
    the finalization event is still on its way is accepted; before, it
    got HTTP 400 (ADR 0007, amended;
    [TD-44](RISKS_AND_TECHNICAL_DEBTS.md#td-44)). A transfer request
    that is refused because it names no finalized agreement of the
    caller is now logged: `transfer request is refused; answered 400`
    (INFO).
12. The management OpenAPI files of the three services list HTTP 403 on
    every route that needs the API key. No service answers 403 in 0.13:
    it is reserved for 1.x (ADR 0025).
13. **Behaviour change, management API clients: an empty id is
    refused.** `datasetId` of `POST /v1/negotiations` (connector) and
    `holderPid` of `POST /v1/credential-requests` (identity) that are
    empty or only spaces answer 400. Leave `holderPid` out if the
    identity should make a new id; before, an empty one did that too.

Rollback to 0.12: put the old names back in the configuration and the
old field names in your clients; for the database, as above.


### From 0.13 to 0.14

0.14 is the release candidate for 1.0. No setting and no route changed
([STABILITY](STABILITY.md)), the databases need no manual step, and no
client has to change. Two checks are stricter; look at them before you
start the new images.

1. **Behaviour change, all three: an item of a list setting may be at
   most 4096 bytes.** This holds for every comma-separated `PORTITOR_*`
   setting (DIDs, DCP scopes, credential types, file paths;
   [DEPLOYING](DEPLOYING.md#settings-of-every-component)). A service
   with a longer item does not start; the log line names the setting and
   the item's position and length, never its text (`has an item that is
   too long: item 2 is 4106 bytes ...`). Real values
   are far shorter, so a long item is most often a missing comma.
2. **Behaviour change, connector management API: the catalog must be
   the provider's own.** `POST /v1/catalog/request` answers 502 in two
   more cases: the provider's catalog has no `participantId` (DSP 2025-1
   requires it; 0.13 handed such a catalog on), or its `participantId`
   differs from the `counterPartyId` of the request. The two are compared
   as written, except that `%3A` and `%3a` in the host of a `did:web`
   are the same. There is no setting to turn the check off. The `detail`
   says which case it is. Set `counterPartyId` to the id the provider names in its
   catalog. `POST /v1/catalog/dataset/request` did not change: a dataset
   has no `participantId`.

To know: the metric `portitor.outbox.message.given_up` no longer counts
a finalization event that the consumer's transfer request had already
settled ([ADR 0007](adr/0007-outbox-and-proof-of-delivery.md), note of
0.14). It also no longer counts a message whose outcome could not be recorded:
that message is still pending and is sent again after a restart. An
alert on that metric may fire less often; an INFO log line says when a
message is not counted.

Rollback to 0.13: as above; 0.14 adds no migration.
