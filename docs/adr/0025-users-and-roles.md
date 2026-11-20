<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0025: Users and roles

- Status: **Accepted** (2026-11-20, by the maintainer; see
  [Answers of the maintainer](#answers-of-the-maintainer-2026-11-20))
- Date: 2026-11-18
- Scope: research only, no code (ROADMAP 0.11, "Research, docs only, in
  0.11 and 0.12"). Answers [TD-04](../RISKS_AND_TECHNICAL_DEBTS.md#td-04)
  and the audit question of [TD-31](../RISKS_AND_TECHNICAL_DEBTS.md#td-31).
  Touches [TD-02](../RISKS_AND_TECHNICAL_DEBTS.md#td-02), which keeps its
  own ADR. Builds on ADR 0005 (bundle), ADR 0013 (data plane), ADR 0016
  (DSP caller authentication), ADR 0017 (Simpl-Open), ADR 0019
  (credential constraints) and ADR 0024 (several data spaces).
- Question asked by the maintainer: how do users identify themselves and
  get their credentials validated, especially in the data plane?
- Decisions already taken by the maintainer: this ADR is decided before
  the 0.13 API and configuration freeze, code comes in 1.5 (ROADMAP,
  2026-11-15 and 2026-11-17); each format is taken from a specification,
  EDC or Simpl-Open, never invented (ROADMAP, "After 0.13").

## Context

Every credential in Portitor is system to system: a participant is an
organisation with one DID (ARCHITECTURE, "Who holds the credentials").
People and applications inside an organisation are not identified:

| Where | Today | Who is the "user" there |
|---|---|---|
| Management API of the connector, the identity and the issuer | One key per component, sent as `Authorization: Bearer <key>` (`PORTITOR_MANAGEMENT_API_KEY_FILE`); the key holder may do everything (0.11 review, F-03). Routes answer 200 to 204, 400, 401, 404, 409, 5xx; never 403 | The participant's **own** operators and applications |
| DSP endpoints | A DCP token of the calling participant on every request (ADR 0016) | No user: connector to connector |
| Data plane, provider side | `GET /data/{flowId}` with the transfer's bearer token, 32 random bytes; every refusal is the same 401 (ADR 0013) | A person or application of the **other** organisation (the consumer) |
| Data plane token, consumer side | `GET /v1/transfers/{consumerPid}` returns the `dataAddress` with the token to whoever holds the management key | The consumer's own people and applications |

So there are two different questions. (1) How do a participant's own
people reach its own Portitor? The organisation controls both ends.
(2) How does a provider know which person of the consumer pulls data?
Here the provider would have to trust an identity the consumer's
organisation gives out.

## What the specifications say (checked 2026-11-18)

| Source | What it says | Link |
|---|---|---|
| DSP 2025-1, terminology | A Participant "registers Participant Agents that perform tasks on its behalf". No user, person or role is defined | [terminology.md](https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/blob/d7d810b276f5f5710ff7a53bff630e2845cc0105/specifications/common/terminology.md) |
| DSP 2025-1, agreement | "An Agreement MUST contain an `assigner` and `assignee`. The contents of these properties are a dataspace-specific unique identifier of the Agreement parties" | [contract.negotiation.protocol.md](https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/blob/d7d810b276f5f5710ff7a53bff630e2845cc0105/specifications/negotiation/contract.negotiation.protocol.md) |
| DSP 2025-1, transfer | "The specification of data plane interfaces and interaction patterns are not in scope"; the data address carries "an opaque authorization token" | [transfer.process.protocol.md](https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/blob/d7d810b276f5f5710ff7a53bff630e2845cc0105/specifications/transfer/transfer.process.protocol.md) |
| ODRL Information Model 2.2 (W3C Recommendation, 2018-02-15) | A rule has an `assigner` and an `assignee` (2.3.1). A `PartyCollection` is "a single entity representing a set of member entities", with a `source` and `refinement` constraints that apply "to the characteristics of each member of the collection" (2.3, 2.5.6; example 17 limits `http://example.com/user44/friends` by `foaf:age`). A left operand "MUST clearly be defined" by a vocabulary or profile (2.5.1). ODRL says nothing on how a party or a membership is proven | [odrl-model](https://www.w3.org/TR/odrl-model/) |
| RFC 9068, JWT profile for OAuth 2.0 access tokens (2021-10) | Required claims `iss`, `exp`, `aud`, `sub`, `client_id`, `iat`, `jti`; header `typ` must be `at+jwt`; the resource server rejects a token whose `aud` is not itself; keys come from the issuer's `jwks_uri` (RFC 8414 or OpenID Connect Discovery). `scope` is the claim for scopes; `groups`, `roles` and `entitlements` may be used with the names of RFC 7643, with no fixed values | [RFC 9068](https://www.rfc-editor.org/rfc/rfc9068.html) |
| OpenID for Verifiable Presentations 1.0 | Final since 2025-07-09 (approved 2025-07-10). Query language DCQL. Formats: W3C VC, ISO mdoc, SD-JWT VC (appendix B). The wallet returns a `vp_token`; it "should not" return an access token. Built for a wallet in front of a person | [final spec](https://openid.net/specs/openid-4-verifiable-presentations-1_0-final.html), [approval](https://openid.net/openid-for-verifiable-presentations-1-0-final-specification-approved/) |
| EUDI Wallet ARF 3.0.0 (2026-07-21) | Wallet units support ISO/IEC 18013-5 and SD-JWT VC; W3C VCDM 2.0 is "optional and meant for non-qualified EAAs only" (5.4.1). Remote presentation uses OpenID4VP with the HAIP profile (5.7). Each relying party "will register itself with a Registrar in its Member State" and needs an access certificate "to authenticate itself towards Wallet Units" (6.4.2) | [chapter 5](https://eudi.dev/latest/main/05-data-model-and-data-exchange-protocols/), [chapter 6](https://eudi.dev/latest/main/06-trust-model/) |
| Gaia-X ICAM 25.11, semantic model | A **Natural Person Party Credential** "is issued by a Participant to entitle a natural person (usually one of his users/employees) to interact with other Relying Parties belonging to other Participants" (6.2.3.1). A Party Credential may carry `gx:identityRoles` ("to be used in a RBAC context") and `gx:identityAttributes` ("ABAC context"), both optional, and `gx:parentPartyCredential` for delegation (6.2). Credentials with personal data must not be published (6.2.1). No protocol to present them is named; the standards chapter lists OpenID4VP and OpenID4VCI and "does not impose Gaia-X design decisions" | [semantic model](https://docs.gaia-x.eu/technical-committee/identity-credential-access-management/25.11/semantic_model/), [standards](https://docs.gaia-x.eu/technical-committee/identity-credential-access-management/25.11/standards/) |

No specification Portitor follows (DSP, DCP) defines a user, a role, or
how a user's identity crosses from one participant to another.

## What others do (checked 2026-11-18)

### Eclipse EDC (Connector and IdentityHub at tag `v1.0.0-RC7`)

| Topic | What EDC does | Source |
|---|---|---|
| No users | "Individual users don't exist as first-level concept in IdentityHub!" A principal is a participant context, "analogous to a company" | [identity-api.security.md](https://github.com/eclipse-edc/IdentityHub/blob/v1.0.0-RC7/docs/developer/architecture/identity-api.security.md) |
| API key | `web.http.<context>.auth.type=tokenbased` with `auth.key` or `auth.key.alias`; header `x-api-key`. IdentityHub: one API key per participant context | [`TokenBasedAuthenticationService`](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/extensions/common/auth/auth-tokenbased/src/main/java/org/eclipse/edc/api/auth/token/TokenBasedAuthenticationService.java) |
| Delegated authentication | `auth.type=delegated`: a JWT checked against the JWKS of "the third-party IdP" (`auth.dac.key.url`, `auth.dac.cache.validity`, `auth.dac.audience`). Management API v5: `edc.iam.oauth2.issuer`, `edc.iam.oauth2.jwks.url`, `edc.iam.oauth2.jwks.cache.validity` | [`DelegatedAuthenticationExtension`](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/extensions/common/auth/auth-delegated/src/main/java/org/eclipse/edc/api/auth/delegated/DelegatedAuthenticationExtension.java), [`ManagementApiOauth2AuthenticationExtension`](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/extensions/common/api/management-api-oauth2-authentication/src/main/java/org/eclipse/edc/connector/api/management/authn/ManagementApiOauth2AuthenticationExtension.java) |
| Authorisation by scope, not role (decision record 2026-06-06) | The `role` claim and the roles `admin`, `provisioner`, `participant` are removed: "There is no standard JWT claim for roles", and the role claim needed a custom Keycloak extension. Scopes are `management-api[:<resource>]:read\|write\|admin`; `write` includes `read`; `admin` crosses tenants. `sub` is the participant context id, or any subject for an admin token. IdentityHub uses the same form (`identity-api:keypairs:write`, `identity-api:admin`, `issuer-admin-api:admin`). In RC7 the code has `ScopeMatcher` and no `@RolesAllowed` | [decision record](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/docs/developer/decision-records/2026-06-06-scope-based-api-authorization/README.md) |
| Data plane | The HTTP proxy data plane was deprecated (2025-02-07): "Given the availability of mature proxy solutions, we prefer to avoid the overhead of maintaining a custom implementation." The RC7 Connector tree has no data plane module, only Data Plane Signaling clients (`data-protocols/data-plane-signaling`) | [decision record](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/docs/developer/decision-records/2025-02-07-http-proxy-data-plane-deprecation/README.md) |
| Policies | Evaluated on the counter-party: "the counterparty id the dataspace profile extracts from the verified token, e.g. a DID", and its claims. Partner groups (2026-09-22) group participants, not people | [partners record](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/docs/developer/decision-records/2026-09-22-partners-and-partner-groups/README.md) |

### Simpl-Open

Read again today, on top of [ADR 0017](0017-simpl-open.md) (2026-11-13):

| Topic | What Simpl-Open does | Source |
|---|---|---|
| Tier 1, people | Tier 1 is "the identification, authentication and authorisation of the organisation's members (humans or machines)"; the agent connects to the organisation's own system ("EU Login, eID, Microsoft AD, OpenID Connect, etc."). The Tier 1 gateway "enforces Role-Based Access Control (RBAC) policies" and stays "within the organization's internal network" | [Architecture D1.3.4 v0.1, 2.2.2 and 2.2.3](https://ssg-website-ad5a79.pages.code.europa.eu/resources/architecture) |
| Tokens and roles | The authentication provider is Keycloak. "Tier1 Gateway forwards the User JWT token to agent Microservices"; each microservice checks roles and scopes itself (ADR-05 of 2025-05-15; scopes such as `participant:create`) | same document, 2.11.2 |
| Users and roles | The participant manages local and federated end users and their roles in its own agent (requirement SIMPL-1689, "Proposed"). A federated user with no role asks for one; the participant's Tier 1 User and Roles Manager approves (BP03C, "Proposed"). No list of role names was found | [3B.3.1](https://simpl-programme.ec.europa.eu/book-page/3a41-users-and-roles-configuration), [BP03C](https://simpl-programme.ec.europa.eu/book-page/bp03c-end-user-role-request) |
| Own identity provider | Gdynia (4.0, in GitLab since 2026-07-06): "Enable federated end-user authentication through existing external identity providers" | [Gdynia items](https://simpl-programme.ec.europa.eu/development/development-status/simpl-open-roadmap-items-gdynia-4.0) |
| Wallets | Hamburg (4.1, planned October 2026), item "Self Sovereign Identification (SSI) Tier 1": "Enable end users to authenticate using verifiable credentials stored in digital wallets" | [Hamburg items](https://simpl-programme.ec.europa.eu/development/development-status/simpl-open-roadmap-items-hamburg-4.1) |
| Between organisations | The user is not passed on: "all actions performed by John Doe are actually performed by the Simpl-Open Agent of Consumer". Tier 2 carries the participant's identity attributes only (ADR 0017) | Architecture, 2.3 |

### EHDS, TEHDAS2 and HealthData@EU

| Source | What it says | Link |
|---|---|---|
| Regulation (EU) 2025/327, Art. 68(10)(d) | The data permit sets out "the identity of authorised persons, in particular the identity of the principal investigator" | [Art. 68](https://streamlex.eu/articles/ehds-en-art-68/) |
| Art. 73(1) | Data under a permit is accessed only in a secure processing environment (SPE), with: (a) "restriction of access to the secure processing environment to authorised natural persons" listed in the permit; (d) access "by means of individual and unique user identities and confidential access modes only"; (e) "identifiable logs of access to and activities in the secure processing environment", kept "for at least one year". Implementing acts on SPE requirements are due by 26 March 2027 (73(5)) | [Art. 73](https://streamlex.eu/articles/ehds-en-art-73/) |
| TEHDAS2 D7.4, SPE specification (v1.2, 2026-02-24) | "Authorised health data users MUST be strongly identified" (EHDSR-5); "All SPE logs MUST identify the actor" (EHDSR-7); logs kept at least one year (EHDSR-8); identities "unique, persistent, auditable, and non-transferrable", role-based access (OPR-1); multi-factor authentication "should be adopted" (4.1.2); "Federation user identities SHOULD be shared", "a nod of approval towards" eIDAS (FSPER-6). No eIDAS assurance level is named | [D7.4](https://tehdas.eu/wp-content/uploads/2026/06/d7.4-technical-specification-for-health-data-access-bodies-on-the-implementation-of-secure-processing-environments.pdf) |
| TEHDAS2 D7.3, common IT infrastructure (v3.3, 2026-02-24) | HealthData@EU nodes exchange messages over eDelivery (AS4, Domibus) with certificates issued by the Commission. It does not say how data users sign in. It does not mention DSP, connectors or verifiable credentials | [D7.3](https://tehdas.eu/wp-content/uploads/2026/05/d7.3-technical-specification-for-health-data-access-bodies-on-the-implementation-of-the-common-it-infrastructure.pdf) |

In the EHDS the persons are named in the permit and checked **by the
SPE**, where they work on the data. The rules above bind the SPE (run
under the health data access body), not the connector that moves data
from a data holder to it.

## The questions of TD-04

### Authentication

| Caller | Finding |
|---|---|
| Own people and applications, to the management APIs | EDC and Simpl-Open both hand this to the organisation's own identity provider: an OAuth 2.0 access token (JWT) checked against the provider's JWKS. A person signs in with OpenID Connect; an application uses a client credential at the same provider. Neither stores users in the connector |
| The identity component | Same as above. It holds the organisation's private key, so it is the API where a person behind each call matters most |
| A consumer's person, to the provider's data plane | No specification, and neither EDC nor Simpl-Open does it. The provider would need to trust the consumer's identity provider (no data space specification says how) or a credential the consumer issues to its user (Gaia-X has the vocabulary, no protocol). In Simpl-Open the user's actions count as the agent's. In the EHDS persons are checked in the SPE |
| A wallet (OpenID4VP, EUDI) | Meant for a person in a browser or on a phone, not for an application that pulls data. A verifier of EUDI person data must be a registered relying party with an access certificate, and must read SD-JWT VC and ISO mdoc; Portitor reads W3C VC 2.0 as JWT only (ADR 0016) |

Link to the participant's DID: with an identity provider, the link is
the operator's configuration (this connector trusts this issuer). With
a Gaia-X Natural Person Party Credential, the link is the credential's
issuer, which is the participant.

### Authorisation

- **Who may do what, inside the organisation.** EDC's scopes fit
  Portitor's routes as they are: `GET` is read, `POST` and `DELETE` are
  write. One point is specific to Portitor: reading a transfer also
  gives the data token (`dataAddress`), so "may see transfers" and "may
  read the data" are one right today. `dataAddress` is optional in the
  API schema, so a later release can leave it out for a caller without
  the right, with no break.
- **Can a contract be narrowed to users or roles?** Not with what
  exists. In DSP `assigner` and `assignee` name the agreement's parties,
  the participants. ODRL could name a `PartyCollection` with a
  `refinement`, but no DSP profile, EDC or Simpl-Open uses it, ODRL does
  not say how membership is proven, and no left operand for a user's
  credential or role was found in DSP, EDC, Simpl-Open or Gaia-X. Gaia-X
  defines `gx:identityRoles` as a claim, not as an ODRL operand.
  Inventing one is ruled out by the maintainer.
- **What works today.** A provider narrows an offer by the consumer
  **participant's** credentials (ADR 0019), for example a credential
  that states a permit. Which of its people use the data is the
  consumer's duty under the contract, enforced on the consumer's side.
- Until 0.11 Portitor dropped `assignee` on a rule and other unknown
  policy terms without an error
  ([TD-32](../RISKS_AND_TECHNICAL_DEBTS.md#td-32)). From 0.12 it refuses
  them (ADR 0009, amendment of 2026-11-18). This had to be in before
  1.0: otherwise a 1.0 connector would silently agree to an offer that a
  later release, or another connector, narrows to users.

### Governance

A data space authority may want to set: the accepted identity providers
or wallets, the roles, the credentials a user needs, and what is logged.
No format for these rules was found: not in DSP or DCP, not in EDC, not
in Simpl-Open (roles are managed by each participant; the authority
defines the participants' identity attributes). Gaia-X ICAM describes a
Trust Scope Credential that names accredited issuers of Party
Credentials (6.1; read in summary only). For the EHDS the SPE rules come
by implementing act, not yet adopted.

The signed bundle (ADR 0005) is the channel Portitor has. But its
manifest is closed: `additionalProperties: false`, and a file's role is
one of `datasets`, `messages`, `vocabulary`, `context`
(`docs/schemas/bundle-manifest.schema.json`; the Rust type refuses
unknown fields). A 1.0 connector refuses a bundle with a role it does
not know. So an authority that adds a rules file later breaks every
older connector in its data space. The same wall is hit first by the
policy shapes planned for 1.2
([TD-32](../RISKS_AND_TECHNICAL_DEBTS.md#td-32)).

Until a data space publishes user rules in a format we can copy, they
are local settings of the operator.

### Audit (TD-31)

- What a record holds: time, component, action (the route or the state
  change), result, the actor (which key, or a token's `iss` and `sub`),
  the process ids, the agreement id and the counter-party's DID. For a
  data pull: the flow id and the bytes sent. Never the token.
- Where it goes: a JSON log line with a fixed marker, on stdout with the
  other logs (ADR 0023). The operator ships and keeps it. Portitor adds
  no audit store.
- How long: the operator's choice. The one-year rule of Art. 73(1)(e)
  binds SPE logs. No EHDS rule for a data holder's connector was found.
- With one key per API, the actor is "the key". A person appears only
  with option A (in the proxy's log) or option B (in Portitor's log).

### Several data spaces

User rules are per data space. With ADR 0024 option (a), the proposed
choice for 1.0, one connector serves one data space, so per-connector
settings are per data space. With option (b) they belong to the data
space profile. The management API is not per data space: it is the
organisation's own door, so who may operate the connector is one rule
set per connector in both cases.

## Options

### A. Stay system to system; users stop at the organisation's own gateway

Portitor keeps one key per management API. The organisation puts its own
reverse proxy or API gateway in front of the management port. The proxy
signs people in with OpenID Connect at the organisation's identity
provider, decides by path and method who may call what, logs the person,
and adds the Portitor key. One documented pattern: nginx `auth_request`
or Traefik `ForwardAuth` calling oauth2-proxy's `/oauth2/auth`, which
answers 202 or 401
([oauth2-proxy 7.6 integration](https://oauth2-proxy.github.io/oauth2-proxy/configuration/integration)).
On the consumer side the same gateway, or the organisation's own
application, pulls the data and gives it to its users. Nothing changes
at the provider's data plane.

| | |
|---|---|
| Cost | None in Portitor: a DEPLOYING section with one worked example |
| Risk | The proxy must be the only way to the management port. Portitor's logs do not name the person; the proxy's log does, matched by time and trace id (ADR 0023). The proxy cannot hide the `dataAddress` field from a read-only user |
| Small, simple, few components | Unchanged. The operator already runs a reverse proxy for TLS |
| EDC, Simpl-Open | EDC's default is the same (`x-api-key`), and it sends data plane users to "mature proxy solutions". Simpl-Open's Tier 1 gateway is this pattern as a product |
| API and configuration | None |

### B. Portitor checks access tokens of the organisation's identity provider

The management API of each component also accepts a JWT access token:
signature against the provider's JWKS, `iss`, `aud`, `exp` (RFC 9068).
What the caller may do comes from the `scope` claim, in EDC's form
(`management-api:read`, `:write`, `:admin`; `identity-api:...`,
`issuer-admin-api:...`). The key file stays and keeps every right. The
audit record names `iss` and `sub`.

| | |
|---|---|
| Cost | Medium: a JWKS client with a cache, token checks, a scope check per route, a fuzz target for the new parsers, tests with one identity provider. JWS verification exists already (DCP). No new component: the identity provider is the organisation's own |
| Risk | A second way in to keep secure. Identity providers differ in the claims they send (`scope` or `scp`, `aud` forms); EDC had to drop its `role` claim for this reason. A wrong `aud` check lets a token for another service in |
| Small, simple, few components | Still one binary and no user store. Three or four new optional settings per component |
| EDC, Simpl-Open | EDC does exactly this (delegated authentication, scopes). Simpl-Open does it with Keycloak roles and scopes |
| API and configuration | Additive: the same `Authorization: Bearer` header, new optional settings, a `403` answer for a token without the scope |
| On the provider's data plane | Not useful: a provider cannot check tokens of each consumer's identity provider, and no specification tells it which provider to trust |

### C. Verifiable credentials for users, presented with OpenID4VP

A person presents a credential from a wallet: a Gaia-X Natural Person
Party Credential issued by the participant, or EUDI person data. The
verifier is Portitor, or a verifier in front of it that turns the
presentation into an access token (then Portitor needs only option B).

| | |
|---|---|
| Cost | Large in Portitor: an OpenID4VP verifier (requests, DCQL, sessions, a browser flow), and for the EUDI Wallet SD-JWT VC and ISO mdoc, X.509 trust lists, registration as relying party in a Member State and an access certificate |
| Risk | No data space profile says which user credential a provider must accept, so two connectors would not agree. A wallet needs a person: it does not fit an application pulling data. A user credential shown to another organisation is personal data crossing the contract |
| Small, simple, few components | Against it, if built in. Neutral, if the organisation's identity provider or a separate verifier accepts wallets and Portitor sees a normal access token |
| EDC, Simpl-Open | EDC: nothing. Simpl-Open: planned for Tier 1 (Hamburg), inside the organisation, in front of the agent |
| API and configuration | None now. Later: new routes on a public port, which are additive |

## Decision (proposed)

1. **For 1.0: option A.** Portitor stays system to system. DEPLOYING
   gets a section "Users and roles" with the reverse proxy pattern, on
   the provider and the consumer side. No code.
2. **For 1.5: option B on the three management APIs,** behind its own
   code ADR, with EDC's settings and scope form as the model. The API
   key stays valid and keeps every right, so nothing breaks. One scope
   decides whether a caller gets the `dataAddress`.
3. **The provider's data plane does not identify persons in 1.x.** The
   contract is between participants; the consumer answers for its
   people. This is what DSP defines, what EDC and Simpl-Open do, and
   where the EHDS puts the check (the SPE). What the data plane should
   re-check on each pull about the **participant** (agreement, policy,
   credential status) is TD-02's ADR.
4. **Not option C in Portitor.** A wallet sign-in can sit at the
   organisation's identity provider and reach Portitor as an option B
   token. Revisit when a data space Portitor joins publishes a profile
   for user credentials.
5. **No ODRL term for users or roles.** Offers are narrowed by the
   participant's credentials (ADR 0019). Revisit when DSP, EDC,
   Simpl-Open or a data space defines such a term.
6. **Audit records as log lines** with the fields above, in 1.5 with
   option B; a data pull line can come earlier, as it needs no user.

Reasons: the organisation already has an identity provider and a
reverse proxy, so A costs nothing and adds no component. B is what EDC
and Simpl-Open both converged on, it is additive, and it gives the two
things a proxy cannot: the person in Portitor's own audit record, and
the data token kept from read-only callers. C and user terms in ODRL
have no specification to follow yet.

## What must be settled before the 0.13 freeze

Nothing in the routes, the DSP endpoints, the data address or the
existing settings has to change. Four small points do:

1. **Say that both bearer values are opaque.** The management key and
   the data plane token are strings a client must not parse; 1.x may
   accept or issue other forms (a JWT). DSP already calls the data
   token opaque.
2. **Say that a management call may be answered `403`.** No route
   documents it today. Add it to the three management OpenAPI files, or
   to the 1.0 stability statement, so option B adds no undocumented
   status code. `PORTITOR_MANAGEMENT_API_KEY_FILE` keeps meaning "every
   right".
3. **Keep `dataAddress` optional** in `TransferView`. It is today.
4. **Decide how a 1.0 connector treats a bundle file with a role it
   does not know** (refuse the bundle, as now, or skip the file with a
   warning). This is the only format question found. It is shared with
   the 1.2 policy shapes and with
   [TD-01](../RISKS_AND_TECHNICAL_DEBTS.md#td-01).

Already decided elsewhere and needed here: the strict reading of policy
terms (TD-32), done in 0.12. Not needed now: any new environment variable.
User settings come in 1.5, per connector or per data space profile (ADR
0024).

## Answers of the maintainer (2026-11-20)

The maintainer accepted the decision above and answered the open
questions below. The numbers are those of the questions.

1. **Option A for 1.0.** Portitor stays system to system; users stop at
   the organisation's own reverse proxy.
2. **For 1.5 the plan is option B,** behind its own code ADR.
3. **Scopes, with EDC's scope names.** They are named in that code ADR.
4. **The provider's data plane does not identify persons in 1.x.**
5. **A bundle file with a role the connector does not know: skip the
   file with a warning. If no file with a known role is left, refuse
   the bundle.** Point 4 of "What must be settled before the 0.13
   freeze" has the details.
6. **`403` goes into the OpenAPI files now.** Done in 0.13 for the three
   management APIs: every route that needs the key lists it as
   reserved. 1.0 never answers it.
7. **The data pull audit line** (flow, agreement, consumer DID, bytes)
   goes on the roadmap for 1.1 or 1.2.
8. **User-to-system access can be tested against Simpl-Open,** which has
   implemented user-to-system actions. This is the maintainer's
   statement; it was not checked for this ADR. What was verified about
   Simpl-Open is in [ADR 0017](0017-simpl-open.md) and in the table
   above.

Freeze points 1 and 3 (opaque bearer values; `dataAddress` stays
optional) are stated in [STABILITY](../STABILITY.md), in the README and
in DEPLOYING.

## Open questions for the maintainer

All eight were answered on 2026-11-20, see above; kept as asked.

1. Is option A enough for 1.0? Recommendation: yes.
2. Should 1.5 build option B, or stay with A until a user asks?
   Recommendation: build B, for the audit record and the `dataAddress`
   right; it is the smallest step that closes finding F-03.
3. Scopes as in EDC (`management-api:read|write|admin`), or roles?
   Recommendation: EDC's scopes, with one more for the data token, named
   in the code ADR after checking EDC's names then.
4. Is it accepted that the provider's data plane never knows the person
   in 1.x? Recommendation: yes; say so in ARCHITECTURE, "Who holds the
   credentials".
5. Unknown bundle roles in 1.0: refuse or skip? Recommendation: skip the
   file with a warning and a metric. An authority cannot upgrade all its
   members at once, and a 1.0 connector cannot enforce the new rules
   either way. Decide with the 1.2 policy shapes research.
6. `403` in the OpenAPI files now, or one sentence in the stability
   statement? Recommendation: the OpenAPI files; it is a docs change.
7. May the data pull audit line (flow, agreement, consumer DID, bytes)
   come before 1.5? Recommendation: yes, in 1.1 or 1.2; it needs no
   user.
8. Is there a data space with real user rules to test against (the
   ICUdata4EU demo, an HDAB's SPE)? Without one, C and the bundle rules
   stay on paper.

## What was checked (2026-11-18)

- In the cloned sources: EDC Connector `v1.0.0-RC7` (commit `3ac4837`)
  and IdentityHub `v1.0.0-RC7` (commit `003d4e8`): the files linked
  above.
- In this repository: the four OpenAPI files in `docs/api/` (0.11.0),
  `docs/schemas/bundle-manifest.schema.json`, the bundle `Role` type in
  `crates/portitor-dcp/src/bundle.rs`, ADR 0005, 0013, 0016, 0017,
  0019, 0023 and 0024, TD-02, TD-03, TD-04, TD-31 and TD-32.
- Online, each at the link given in the tables: DSP 2025-1 (commit
  `d7d810b`), ODRL Information Model 2.2, RFC 9068, OpenID4VP 1.0 Final,
  EUDI ARF 3.0.0 chapters 5 and 6
  ([releases](https://github.com/eu-digital-identity-wallet/eudi-doc-architecture-and-reference-framework/releases):
  v3.0.0, 2026-07-23), Gaia-X ICAM 25.11 (semantic model, standards,
  [digital identities](https://docs.gaia-x.eu/technical-committee/identity-credential-access-management/25.11/digital_identities/),
  [glossary](https://docs.gaia-x.eu/glossary/25.11/natural_person_party_credential)),
  the Simpl-Open architecture document and programme pages, Regulation
  (EU) 2025/327 Art. 68 and 73, TEHDAS2 D7.3 and D7.4, oauth2-proxy 7.6.

## Not verified

- The EHDS articles were read on streamlex.eu, a third-party copy;
  EUR-Lex returned only the recitals here. Art. 61, 67 and 105 (duties
  of data users, the application, dates of application) were not read.
- The draft implementing regulation on HealthData@EU (consultation to
  29 September 2026) and Implementing Regulation (EU) 2026/2099 on
  identification in MyHealth@EU (primary use) are known from news
  articles only
  ([LexisNexis](https://lexisnexis.co.uk/legal/news/commission-consults-on-technical-requirements-for-healthdata-eu),
  [Biometric Update](https://www.biometricupdate.com/202609/eu-sets-digital-id-rules-for-cross-border-health-data-exchange)).
  How a data user signs in to HealthData@EU or to a national SPE was
  not found.
- TEHDAS2 D7.3 and D7.4 and the Simpl-Open architecture document: the
  last part of each (after 100,000 characters) was read only in part.
- Whether Simpl-Open Hamburg (4.1) is released, what SSI Tier 1 uses on
  the wire, and the names of Simpl-Open's Tier 1 roles.
- What an EDC data plane outside the Connector repository checks on a
  pull, and the Data Plane Signaling specification itself (only EDC's
  client modules were read).
- Gaia-X ICAM 25.11: the Trust Scope Credential (6.1) and the employee
  example (6.5) were read in summary, not in full. That no ODRL left
  operand for users exists in Gaia-X is "none found", not proven.
- RFC 6750, RFC 7643, RFC 8414, OpenID Connect Core and Discovery, HAIP
  and the status of OpenID4VCI were not read; only RFC 9068 and
  OpenID4VP were.
- That an identity provider can accept a wallet presentation and issue
  a normal access token (decision 4): not tested, no product checked.
- Option B's cost is an estimate from reading the code, not a spike.

## Consequences

- TD-04 points here; it stays Open until the maintainer decides.
  Update 2026-11-20: decided. TD-04 is Accepted for 1.0; the code of
  option B is planned for 1.5.
- With decision 1: DEPLOYING gets "Users and roles" before 0.13, and
  ARCHITECTURE says that the provider's data plane does not know
  persons.
- With decision 2: a code ADR and the work in 1.5; finding F-03 stays
  open until then, with the proxy pattern as its mitigation.
- The four freeze points are small docs changes, except the bundle
  role question, which may change `portitor-dcp`'s manifest reader.
- TD-02 keeps its own ADR: what is re-checked about the participant on
  each data pull.
- Re-check Simpl-Open (SSI Tier 1), the EHDS implementing acts and the
  Gaia-X ICAM release before the 1.5 code ADR.
