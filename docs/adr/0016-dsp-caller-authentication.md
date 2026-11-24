<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0016: DSP caller authentication with DCP

- Status: **Accepted** (2026-11-13, by the maintainer)
- Date: 2026-11-13
- Scope: release 0.5. Closes the known gap "no DSP caller authentication"
  open since 0.3. Builds on ADR 0013 (agreement identity check) and
  ADR 0015 (identity component).
- Decisions already taken by the maintainer (2026-11-13, 0.5 plan): the
  provider requires one `MembershipCredential` from an issuer in a
  configured trusted list on every DSP request; credential rules in ODRL
  wait for 0.6; `/.well-known/dspace-version` stays open; the connector
  gets its own tokens from the identity component's STS.

## Context

Until 0.4 the connector believes any caller. The participant id in a
message, and so the agreement's `assignee`, is whatever the caller wrote.
ADR 0013 already checks that later transfer messages match the agreement,
but the identity behind them is not proven.

DCP v1.0.1 (base protocol, section 4.3; presentation protocol, section 5)
gives the flow. When connector A calls connector B:

1. A asks its STS for a self-issued ID token: `iss` = `sub` = A's DID,
   `aud` = B's DID, and a `token` claim, an access token that lets B read
   some of A's credentials (the scopes).
2. A sends the DSP request with `Authorization: Bearer <ID token>`.
3. B resolves A's DID document, checks the token (signature with a key
   that has `capabilityInvocation`, `aud`, `exp`, `nbf`, `jti` not seen
   before, `iss` = `sub`).
4. B finds A's Credential Service in A's DID document and sends it a
   `PresentationQueryMessage` with the scopes, using its own ID token
   (from B's STS) that carries A's access token.
5. B checks the presentation and each credential: signature, holder is
   A, issuer trusted, not expired, not yet valid, not revoked
   (BitstringStatusList), the requested credentials all present.

Checked in EDC (Connector tag v1.0.0-RC7, 2026-11-13): the DSP
dispatcher sends `Authorization` with the token, and `DcpIdentityService`
adds the `Bearer ` prefix when sending and refuses a token without it.
The DCP TCK verifier tests (v1.2.1) start the flow with a DSP catalog
request carrying `Authorization: Bearer <JWT>` and check every rule in
steps 3 and 5, including revocation.

## Decision

### Which requests need a token

- **Every DSP endpoint**, provider and consumer side (catalog,
  negotiation, transfer, and the consumer callbacks), refuses a request
  without a valid token and the required credentials.
- **Open**: `/.well-known/dspace-version` (discovery, DSP 2025-1) and the
  data plane endpoint, which keeps its own short-lived token (ADR 0013).
- A refused request gets HTTP 401 with a DSP error body and no detail on
  which check failed (the log has the detail).

### What a provider (or consumer) requires in 0.5

- One credential of type `MembershipCredential`, in either DCP profile,
  issued by a DID in `PORTITOR_TRUSTED_ISSUERS` (a list of DIDs). No
  trusted issuers configured means the connector does not start.
- Scopes are configuration, with a default of
  `org.eclipse.dspace.dcp.vc.type:MembershipCredential` (the TCK's
  default; EDC uses its own alias, so the EDC test sets it explicitly).
  DCP leaves scope values to the dataspace.
- The consumer side checks the provider the same way on callbacks, so a
  third party cannot push state changes into our processes.

### The caller's identity

- The verified DID (the token's `sub`) is the caller's participant id.
  A message whose participant fields name another DID is refused.
- Each negotiation and transfer stores the counter-party DID when it
  starts. Every later message on that process must come from the same
  DID. This replaces "trust the caller" in the ADR 0013 agreement check
  with a proven identity.
- `PORTITOR_PARTICIPANT_ID` must equal the identity component's DID; the
  connector checks it at start against the DID document.

### Outgoing requests

- Before each DSP request the connector asks its STS for a token with
  `aud` = the counter-party's DID and the configured scopes. The STS URL
  and client secret file are configuration
  (`PORTITOR_STS_URL`, `PORTITOR_STS_CLIENT_SECRET_FILE`). An external
  wallet with a compatible STS (for example EDC IdentityHub) works too.
- The counter-party's DID comes from the negotiation or from the
  management API call that starts it, never from an incoming message.

### Cost and caching

- Each DSP request costs one DID resolution, one token from our STS and
  one presentation query. DID documents are cached for a short time
  (default 5 minutes). Presentations are not cached in 0.5: revocation
  must take effect at once. Measure before adding more caching. Status
  lists are cached from 0.11 (amendment below).

### Security

- DID resolution and presentation queries fetch URLs that the caller
  chooses (its DID). Rules: HTTPS only (plain HTTP only with
  `PORTITOR_ALLOW_HTTP_CALLBACKS`, for tests), time and size limits on
  every fetch, no redirects. The SSRF block for internal addresses with
  an operator allow list (roadmap, later) applies here too.
- Token ids (`jti`) are remembered until expiry to refuse replays. With
  one replica per SQLite file this list lives in memory.
- Parsing of tokens, DID documents, presentations and status lists is
  covered by fuzz targets (ClusterFuzzLite).

### Amended during the 0.5 review (2026-11-13)

The decision stands; one detail changed while it was built:

- The `tck` build maps the DSP TCK's fixed `Authorization` header to the
  participant id `TCK_PARTICIPANT` (the TCK's
  `DspConstants.TCK_PARTICIPANT_ID`), not to a test DID: that id is
  what the TCK's own messages and agreements use.

### Amended in 0.11: status lists are cached (2026-11-17)

The maintainer decided to amend this ADR, not to write a new one
(ROADMAP 0.11, "Decisions on the 0.11 plan", 2026-11-17). It changes
"Cost and caching" above for status lists only:

- The verifier reuses a status list credential for
  `PORTITOR_STATUS_LIST_CACHE_SECONDS` (default 300, at most 3600; `0`
  fetches it on every check), keyed by its URL, in memory.
- Only a list that was fetched and verified (signed by the credential's
  issuer, valid now) is kept. A failed fetch or check is never kept.
- A list is never kept past its own end (`validUntil`, or the JWT's
  `exp`).
- The cost: a revocation takes up to that long to be seen (DEPLOYING,
  "Status list cache"). Eclipse EDC 1.0.0-RC7 caches status lists for 15
  minutes by default (`edc.iam.credential.revocation.cache.validity`).
- Presentations and tokens are still not cached: each DSP request still
  costs one DID resolution (cached as before), one STS token and one
  presentation query. Whether to cache them is decided after measuring
  ([TD-07](../RISKS_AND_TECHNICAL_DEBTS.md#td-07)).
- The demos, the EDC interop test and the TCK runs set `0`, so their
  revocation steps see a revocation at once.

### Amended in 0.11: a check that could not finish answers 503 (2026-11-17)

The 0.11 engineering review found that every failed check answered 401,
also when a service the check needs was down. From 0.11 a partner's
outbox takes a 401 as final (ADR 0007, amended in 0.11), so a short
outage at the partner's issuer would end a negotiation. The review
recommended this change, and the maintainer chose it on 2026-11-17
over two other options (retry a 401 in the outbox, or keep 401 final).
It changes "Which requests need a token" above:

- The check answers HTTP 503 with a problem body, and no reason, when it
  could not finish: a fetch it needs got no answer (connection failed,
  timeout), or the answer 408, 429 or 5xx. The fetches are the caller's
  DID document and Credential Service, the issuer's DID document and
  status lists, and our own STS. A check that takes longer than its
  deadline (20 s) answers 503 too.
- Every other failure still answers 401: no token or a bad one, a
  replayed token, wrong audience, an untrusted issuer, a revoked or
  invalid credential, and also a 404 or an invalid document from the
  caller's or the issuer's server. Those do not change when sent again.
- The log line for a 503 is `DSP request not checked: a service the
  check needs is unavailable; answered 503` (WARN), with the reason.

### Amended in 0.12: three DCP details, recorded (2026-11-18)

The review of 2026-11-18 (finding C2,
[TD-37](../RISKS_AND_TECHNICAL_DEBTS.md#td-37)) found three choices of
the verifier that were not written down. None changes; each is recorded
here with its reason.

- **The caller's ID token must carry the `token` claim.** DCP v1.0.1
  makes the claim optional (section 4.3.1: a Self-Issued ID Token "MAY
  contain an access token as a `token` claim"). The verifier refuses a
  token without it, or with an empty one
  (`crates/portitor-connector/src/auth/dcp.rs`, `check_token`). Reason:
  every DSP request needs the caller's credentials ("What a provider
  requires" above), and the access token is the only way to ask the
  caller's Credential Service for them (section 5.1, step 6). Without
  it the check cannot be done, so the request is refused (401 on the
  catalog routes, 404 on the others, see below). Eclipse
  EDC Connector 1.0.0-RC7 does the same: `TokenNotNullRule` ("The
  'token' claim is mandatory and must not be null") is a rule of every
  self-issued token (`DcpCoreExtension`), and `DcpIdentityService`
  sends the claim's value to the caller's Credential Service.
- **Presentations are read as JWT strings only.**
  `presentation-response-message-schema.json` also allows a JSON object
  in `presentation`. Portitor supports the two DCP profiles,
  `vc11-sl2021/jwt` and `vc20-bssl/jwt` (DCP appendix A), and both
  secure a presentation as a JWT. An answer with an object is refused
  (`presentation_jwts`). An object would be a presentation with an
  embedded proof (JSON-LD), which no supported profile uses; reading
  one needs a new profile and an ADR.
- **`/.well-known/dspace-trust` is not served.** The path is defined
  only in `specifications/dsp.profile.md` of the DCP repository (tag
  `v1.0.1`). That file is not part of the published specification:
  `index.html` at that tag does not include it. It asks "catalogs that
  support the Credential Issuance Protocol" to answer there; a
  connector does not issue credentials. Eclipse EDC 1.0.0-RC7 does not
  serve it either: the text `dspace-trust` appears in neither the
  Connector nor the IdentityHub source (searched 2026-11-18). What an
  issuer offers is read from its Issuer Metadata API (section 6.7).

### Amended in 0.12: a credential's key must belong to its issuer (2026-11-18)

TD-37 also asked whether Portitor must check "that the issuer's base
URL equals the `issuer` of the credentials it signs". What was found:

- The sentence is in DCP v1.0.1, section 6.3 ("Issuer Service Base
  URL"): "The Credential Issuer will use the base URL for the `issuer`
  field in all Verifiable Credentials it issues". It says what an
  issuer does. It gives no check to a holder or a verifier, and it does
  not fit the rest of the specification, where the issuer is a DID: the
  `issuer` of the issuer metadata and of an offer is "the Credential
  Issuer DID" (sections 6.6.1 and 6.7.1), and the verifier compares a
  DID (section 5.4.3, step 4). EDC IdentityHub 1.0.0-RC7 writes the
  issuer's DID into the credentials it issues and compares no URL
  anywhere. So Portitor has no URL check, and does not add one.
- What the specification and EDC do require is about the issuer's DID,
  and Portitor already did it:
  - verifier: the credential is checked with a key from the DID
    document of its `issuer`, and for VC 1.1 the JWT's `iss` must equal
    `vc.issuer` (`portitor-dcp/src/credential.rs`);
  - holder (Storage API): the delivering issuer is the one the request
    was sent to, each credential's `issuer` is that issuer, and it is
    signed with that issuer's key (`portitor-identity/src/storage.rs`;
    EDC `CredentialWriterImpl` checks the same);
  - holder: an offer's `issuer` is the DID of the token that brought
    it, and the issuer metadata's `issuer` is the DID that was asked
    (EDC checks the first, not the second).
- **One gap, closed in 0.12.** Section 5.4.3, step 4: "The Verifier
  MUST assert that the DID in the `VerificationMethod` of the
  Verifiable Credential has the same value as the `issuer` of the
  Verifiable Credential." A DID document can list a verification
  method of another DID. Until 0.11 a credential signed with such a
  key was accepted when the issuer's document listed it under
  `assertionMethod`. From 0.12 it is refused: the id of the signing
  key's verification method must start with the issuer's DID and `#`,
  or be written as the fragment only (`Credential::verify`, used by the
  verifier and by the holder's Storage API). EDC checks the same on the
  key id: `IssuerKeyIdValidationRule` in the Connector (`kid` starts
  with `iss` and `#`) and the key resolver of `CredentialWriterImpl` in
  IdentityHub.

### Amended in 0.12: the DID document cache time is a setting (2026-11-18)

"Cost and caching" above says DID documents are cached "default 5
minutes", but the time was fixed in the code (0.11 threat model review,
F-04; [R-41](../RISKS_AND_TECHNICAL_DEBTS.md#r-41)). From 0.12:

- `PORTITOR_DID_CACHE_SECONDS` sets how long a resolved DID document is
  reused: default 300, at most 3600; `0` turns the cache off, so every
  use fetches the document. The same rules as
  `PORTITOR_STATUS_LIST_CACHE_SECONDS`; a value outside them stops the
  start.
- It applies to every component that resolves DIDs: the connector (the
  verifier, and the authority's document for the bundle key), the
  identity and the issuer. Each reads the same variable.
- The cost of a long time: a key its owner removed from the document is
  still accepted for up to that long, and a new key may be refused for
  up to that long. The cost of `0`: one more fetch for each DSP request
  (the caller's document), plus one for each issuer of its credentials.
- As before, at most 1,000 documents are kept, and a failed lookup is
  never kept. The readiness check of the connector's own identity
  always fetches (no cache), as before.

### Amended in 0.12: a refused caller gets 404 on negotiation and transfer routes (2026-11-18)

The 2026-11-18 review of specifications and code quality (D5,
[TD-36](../RISKS_AND_TECHNICAL_DEBTS.md#td-36)) found that a refused
caller got 401 on every route. The maintainer decided on 2026-11-18 to
follow the specification and Eclipse EDC. It changes "A refused request
gets HTTP 401" in "Which requests need a token" above:

- On the negotiation and transfer routes, the consumer's callbacks
  included, a refused caller gets **HTTP 404** with the endpoint's DSP
  error (`code` "404", `reason` "unauthorized"). DSP 2025-1, "Contract
  Negotiation HTTPS Binding" and "Transfer Process HTTPS Binding",
  section "Unauthorized Access": "If the client is not authorized, the
  Consumer or Provider MUST return an HTTP 404 (Not Found) response."
- On the catalog routes it still gets **HTTP 401**. The "Catalog HTTPS
  Binding" only asks for "an appropriate HTTP code".
- This is what Eclipse EDC 1.0.0-RC7 answers (`DspRequestHandlerImpl`,
  `unauthorizedStatus`: 401 for a `CatalogError`, 404 otherwise).
- **503 stays** on every route when the check could not finish (the
  0.11 amendment above).
- The reason is still only in the log: `DSP request refused: caller not
  authenticated` (WARN) with `path`, `reason` and the `status` answered.
  No token content is logged. A partner cannot tell a refusal from an
  unknown process by the status, so the operator of the connector that
  answered must read this line (DEPLOYING, "Health and logs").
- For a partner's outbox nothing changes: a 404 ends the tries at once,
  as a 401 did (ADR 0007, amended in 0.11). The `tck` build tries a 404
  again ([R-08](../RISKS_AND_TECHNICAL_DEBTS.md#r-08)); for a refusal
  that only makes it give up a little later.

### Amended in 1.1: a revoked partner may still end a shared transfer (proposed 2026-11-24)

Proposed for [TD-47](../RISKS_AND_TECHNICAL_DEBTS.md#td-47) (maintainer
chose this option, D4, 2026-11-24). Once a partner's
`MembershipCredential` is revoked, the connector refuses all its DSP
messages. That includes the provider's termination of a transfer the
consumer just requested, so the transfer stays `REQUESTED` until the
consumer's operator ends it.

DSP 2025-1 lets either party send a termination "at any point except a
terminal state" and leaves the meaning of tokens to other
specifications ("Transfer Process Protocol"; "Common Functionality",
Authorization). So who may end a process is Portitor's choice.

Change: a `TransferTerminationMessage` or a transfer error is accepted
from a caller whose DCP token is valid and whose DID is the stored
counter-party of that transfer, even when its `MembershipCredential` is
revoked, missing or from an issuer no longer trusted. Every other
message from such a caller is still refused as above (404). A
termination only ends access, so accepting it gives the caller nothing.
The log line says `DSP termination accepted from a partner without
membership` with the process id and the DID.

## Consequences

- Running a connector now needs an identity component (or a compatible
  external wallet) and at least one trusted issuer. `scripts/demo.sh`,
  the Compose demo and the DSP TCK set-up all gain an identity and a test
  credential (test-only tool, ADR 0015).
- The DSP TCK v1.0.2 can only attach one fixed `Authorization` header
  to every request (`dataspacetck.dsp.connector.http.headers.authorization`,
  its README). A DCP token cannot be fixed: it expires and its `jti` may
  be used once. So DSP TCK runs use the `tck` build (like the TCK hooks
  today, never in release images), which maps that fixed header to a test
  DID. The DCP TCK covers the authentication itself, and the EDC test
  covers both together.
- DSP calls get slower by the round trips above. Numbers are measured
  in 0.5 and published with the release (ADR 0002, section 7, "Measured
  again for 0.5").
- Credential rules in policies (for example "member of dataspace X") come
  in 0.6 on top of this, using the verified credentials kept for the
  request.
