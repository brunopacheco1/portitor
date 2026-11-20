<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0005: Schema and vocabulary registry

- Status: **Accepted** (2026-11-15, by the maintainer, with the bundle
  format built in 0.7; how schemas are collected and stored stays open,
  see TD-01 in [RISKS_AND_TECHNICAL_DEBTS](../RISKS_AND_TECHNICAL_DEBTS.md#td-01))
- Date: 2026-11-12, updated 2026-11-14
- Build: release 0.7 (planned for 0.6 with the authority image on
  2026-11-12; moved to 0.7 by the maintainer, 2026-11-14)

## Context

A dataspace authority decides which metadata profiles and vocabularies
the dataspace accepts, for example HealthDCAT-AP for health data.
Participants fetch these rules and check their own dataset descriptions
before they publish them in the DSP catalog. They may also check the
catalogs of other participants. (Requirement from the maintainer,
2026-11-12.)

The same applies to every DSP message, not only catalogs: for example,
the authority may require that offers and agreements in a contract
negotiation follow an ODRL profile (allowed constraints and operands),
published as SHACL shapes. Participants check messages they receive
before processing them, and messages they send before sending them.
(Requirement from the maintainer, 2026-11-12, during 0.3.)

### Facts checked (2026-11-12)

| Fact | Source |
|---|---|
| HealthDCAT-AP is a profile of DCAT-AP, so its data is RDF. Its rules are published as SHACL shapes, including shapes that check values against controlled vocabularies. Release 7 is the latest release we could confirm. | [HealthDCAT-AP release 7 shapes](https://healthdataeu.pages.code.europa.eu/healthdcat-ap/releases/release-7/html/shacl/public-shapes.ttl), [vocabulary shapes](https://healthdataeu.pages.code.europa.eu/healthdcat-ap/releases/release-7/html/shacl/mdr-vocabularies.shape.ttl) |
| A DSP 2025-1 catalog is JSON-LD. The DSP context defines the `dct`, `dcat`, `odrl`, `xsd` and `dspace` prefixes only. Any other prefix, such as a HealthDCAT-AP one, needs an extra `@context` entry. | [DSP 2025-1 context](https://w3id.org/dspace/2025/1/context.jsonld) |
| The Gaia-X Registry publishes credential schemas as SHACL shapes, and the Compliance service validates data graphs against them. The architecture document does not describe how shapes are fetched, versioned or protected. | [Gaia-X Trust Framework architecture 25.11](https://docs.gaia-x.eu/technical-committee/architecture-document/25.11/trust_framework_architecture/) |
| In Simpl-Open, the governance authority manages vocabularies and resource description schemas. Publishing a new schema version revokes the old one for new descriptions; descriptions that already exist stay valid. This business process is marked "Proposed". | [Simpl BP02C](https://simpl-programme.ec.europa.eu/book-page/bp02c-manage-resource-description-schemas), [BP02B](https://simpl-programme.ec.europa.eu/book-page/bp02b-manage-vocabularies) |
| rudof is a Rust library for SHACL, ShEx and RDF, under Apache-2.0 or MIT. It does not claim to be production-ready, and it warns against releases 0.2.19 to 0.3.0. | [rudof](https://github.com/rudof-project/rudof) |

### Criteria

1. Participants can trust that the rules come from the authority and
   have not been changed (integrity and origin).
2. Versions are explicit. A dataset states, or the participant pins, the
   version it follows, and old versions stay readable.
3. Little to run: the authority should not need a new online service if
   it can be avoided.
4. Works with the formats in use: SHACL for shapes, SKOS for
   vocabularies, JSON-LD in DSP.
5. Fits ADR 0003 (images by operator and key) and ADR 0004 (ports and
   adapters).

## Options

### Distribution of the rules

**A. Signed static bundle (recommended).** The authority publishes:
- a manifest listing each shape and vocabulary file with its SHA-256
  digest, plus the bundle's version and validity dates;
- a signature over that manifest, made with the authority's DID key (the
  same key family as the issuer, ADR 0003).

Any HTTPS server can host the bundle. LDSC adds a command to
`ldsc-issuer` that builds and signs it, and a module in `ldsc-connector`
that fetches it, verifies the signature against the authority's
`did:web` document, checks the digests and caches the result.
- Plus: no new online service; integrity and origin are checked; works
  offline from the cache; old versions are plain files.
- Minus: we define our own manifest format (small, documented, with a
  JSON Schema).

**B. A registry service in `ldsc-issuer`.** An HTTP API serves the
shapes and vocabularies.
- Plus: one place to query.
- Minus: an online service that must stay available, with the authority's
  signing key in an internet-facing process. And participants still need
  a way to verify what they receive.

**C. Shapes as verifiable credentials.** Each shape is wrapped in a
credential issued by the authority.
- Plus: reuses DCP issuance.
- Minus: shape files are large and change rarely; credentials are a poor
  fit, and no peer (EDC, Simpl-Open, Gaia-X) does this.

### Validation engine in the connector

**1. In-process with rudof**, behind a `ShapeValidator` port.
- Plus: one process; small image.
- Minus: maturity is unproven. We must also convert JSON-LD to RDF,
  which needs a JSON-LD processor we have not chosen yet.

**2. External validator as a sidecar**, behind the same port: for
example the EU Interoperability Test Bed SHACL validator, or Apache Jena.
- Plus: mature engines.
- Minus: a second container and a JVM, which works against the
  small-footprint goal.

## Decision (proposed)

- Three ports, so each part can be replaced without touching the
  protocol logic (ADR 0004):
  - `MessageGuard` (in `ldsc-dsp`, since 0.3): every DSP message passes
    through it at the HTTP edge, inbound before processing and outbound
    before sending, with its `@type` and the raw JSON-LD. An inbound
    rejection answers HTTP 400 with the protocol's error message and
    changes no state; an outbound rejection is not sent, and the
    negotiation or transfer is terminated with the reason logged. The
    default adapter, `AllowAll`, accepts everything.
  - `BundleSource`: where shapes, vocabularies and JSON-LD contexts come
    from. Adapters: a local folder (development and tests), the signed
    bundle below, later a Gaia-X or Simpl-Open registry if one is
    standardised.
  - `ShapeValidator`: the SHACL engine (see "Validation engine").
- JSON-LD contexts used for validation come from the `BundleSource`
  only; the validator never fetches a `@context` from the network, so a
  counter-party cannot point it at a hostile context.
- Distribution: **Option A**, a signed static bundle.
- Validation: a `ShapeValidator` port. Option 1 (rudof) is the first
  adapter, if a spike at the start of 0.6 shows that it validates
  HealthDCAT-AP release 7 sample data correctly. Option 2 is the
  fallback.
- In 0.7 (updated 2026-11-14 for the maintainer's decisions on dataset
  harvest, ADR 0020):
  - Each harvested dataset is checked against the shapes of the pinned
    bundle for its `dct:accessRights` level. A dataset that fails is not
    published, and the error is stored with the harvest job; the
    connector keeps running. A dataset that passes is published with the
    bundle's profile as `dct:conformsTo`.
  - Checking other participants' catalogs is not in 0.7.
  - The SHACL `MessageGuard` adapter checks DSP messages against the
    bundle's `messages` shapes.

### Bundle format (proposed for 0.7)

Facts checked 2026-11-14: HealthDCAT-AP release 8 publishes its SHACL
shapes in section 15 of its specification
([release 8](https://data.health.europa.eu/healthdcat-ap/releases/release-8/),
CC-BY 4.0). There are shapes and recommended-property shapes for each of
three access levels (public, restricted, non-public), vocabulary shapes,
and `owl:imports` of the DCAT-AP 3.0.0 shapes, ontologies and code
lists, some only as RDF/XML. Only SHACL Core is used, plus two terms
outside it: `sh:extends` (release 8) and `sh:shape` (DCAT-AP 3.0.0,
SHACL 1.2 draft).

- A bundle is a folder: the files, `manifest.json` and
  `manifest.jws`.
- `manifest.json` (JSON Schema in `docs/schemas/bundle-manifest.schema.json`)
  holds: the bundle version (`dcat:version`), when it was issued
  (`dcterms:issued`, an `xsd:dateTime` in UTC to the second), the
  authority's DID, the profile
  IRI used as `dct:conformsTo`, and one entry per file with its path,
  SHA-256, media type (`text/turtle`, `application/n-triples` or
  `application/ld+json`) and role (a role of another type is skipped
  from 0.13, see the amendment below):
  - `datasets`: shapes for dataset metadata, with the `dct:accessRights`
    values they apply to;
  - `messages`: shapes for DSP messages;
  - `vocabulary`: ontologies and code lists, as data for `sh:class`
    checks;
  - `context`: a JSON-LD context, with the URL it stands for. It is used
    only to read DSP messages whose `@context` names that URL (the
    message check); it is never served, and never fetched. A bundle file
    for the DSP 2025-1 context URLs is not used: the connector's own copy
    wins.
- The catalog `@context` stays the DSP 2025-1 context URL alone, with or
  without a bundle (maintainer, 2026-11-14): the DSP JSON Schema
  (`context-schema.json`) allows only URL strings in `@context`, and EDC
  RC7 does not fetch remote contexts by default
  (`edc.jsonld.http.enabled` and `edc.jsonld.https.enabled` are `false`).
  So datasets write every term outside the DSP context's prefixes
  (`dct`, `dcat`, `odrl`, `xsd`, `dspace`) as a full IRI, for example
  `http://healthdataportal.eu/ns/health#healthCategory`, and the SHACL
  check reads each dataset with the DSP context only, as the catalog
  serves it.
- `manifest.jws` is a compact JWS (ES256) over the bytes of
  `manifest.json`, signed with a **bundle key** of the authority: its own
  verification method in the authority's `did:web` document, separate
  from the credential signing key (maintainer, 2026-11-14). Enforced:
  the JWS `kid` must be `<authority>#bundle-key` and its `alg` ES256; a
  connector refuses a bundle signed with any other key of the
  authority, the credential key included (it is also under
  `assertionMethod`), and `bundle build` refuses any other `--kid`.
- Code lists travel as reduced N-Triples (`rdf:type` and
  `skos:inScheme` triples only). The build tool accepts only Turtle,
  N-Triples and JSON-LD contexts, so no Portitor binary parses RDF/XML.
- `portitor-issuer bundle build` signs a folder offline; the issuer can
  also serve a bundle folder on its public port.
- The authority is the source of truth; the connector keeps a read copy,
  the same way it harvests datasets (ADR 0020), and **pulls** new
  versions (maintainer, 2026-11-14). Pull, not push: the connector
  already reaches the authority for its DID document, the authority
  needs no list of connectors, and a connector that was down catches up
  on its next check.
- The bundles folder holds one folder per version and `latest.json`
  (`{"version": "..."}`), the newest version. `bundle build` writes it
  (unless `--not-latest`) and never points it at a version issued before
  the one it names. It is not signed: the connector still checks the
  signature of the version it names.
- The connector follows `latest.json` by default
  (`PORTITOR_BUNDLE_VERSION=latest`), at start and every
  `PORTITOR_BUNDLE_REFRESH_SECONDS` (default 3600, 0 only at start), or
  keeps one fixed version. It verifies the signature against the
  authority's DID document and every digest, and keeps the verified
  bundle in a disk cache, so a restart works while the authority's host
  is down.
- A new version is put in use without a restart. The rules are swapped
  at once; checks going on finish with the old rules. Then the stored
  datasets are checked again and the source is harvested again.
- No rollback: the connector remembers the version in use (in the cache)
  and never takes a version issued before it. A version that cannot be
  taken (unreachable, refused, older) leaves the version in use, and the
  run says why. The order is that of `issued`, the field W3C DCAT 3
  uses for a version's release date (section 11.2, "Version
  information"); we do not define our own version order (maintainer,
  2026-11-14: no new standards). It is a date and time (maintainer,
  2026-11-14), so versions issued on the same day keep their order;
  `bundle build` writes the time of the build by default.
- Compared with Simpl-Open (checked 2026-11-14): its Governance
  Authority's schema-manager numbers versions itself and marks them
  published or revoked; a provider's schema-sync-adapter pulls them over
  REST, is also told by webhook, and syncs again every 900 s. Schemas
  are not signed there. Its version rules ("no gaps, no duplicate
  numbers") are still Proposed requirements.
- Each check is a run in the harvest job log, with the kind `bundle`
  (ADR 0020).
- Technical debt (maintainer, 2026-11-14): how schemas are collected and
  stored is to be investigated further, so that Portitor follows an
  existing standard rather than its own; see TD-01 in
  [RISKS_AND_TECHNICAL_DEBTS](../RISKS_AND_TECHNICAL_DEBTS.md#td-01)
  (moved there from the roadmap's "Technical debt" on 2026-11-15). The bundle source and store are in their own crate,
  `portitor-bundle`, so they can be reused by other services or
  replaced as a whole.
- The engine ignores `sh:extends` and `sh:shape` with a warning when the
  bundle is loaded, and refuses any other term it does not implement.

### Amendment for 0.13: a file with a role the connector does not know is skipped (2026-11-20)

The bundle format is frozen in 0.13 for 1.0, and later releases will add
roles (1.2: policy shapes). Until 0.13 a manifest with a file whose
`role.type` was not `datasets`, `messages`, `vocabulary` or `context`
made the whole bundle invalid: an authority could not add a role before
every member had upgraded. The maintainer decided (2026-11-20;
[ADR 0025](0025-users-and-roles.md), point 4):

- **A connector skips a file whose role type it does not know.** It
  writes one WARN line for each such file, with its path and role type
  (`bundle file skipped: ...`), sets the gauge
  `portitor.bundle.file.skipped` to their number, and the bundle run in
  the job log (`GET /v1/harvest-jobs`) says how many were skipped. The
  other files are used as before.
- **The skipped file is not downloaded, not hashed and not kept in the
  cache.** This is safe, and simpler than fetching it: the signature
  covers the manifest, so the path, digest and role of the skipped file
  are the authority's, and nothing uses its bytes. Not reading bytes of
  a kind the connector does not know is also less to attack, and a
  missing or broken file of a new kind cannot stop a connector that
  would not use it. The cost: such a connector does not notice a broken
  file of the new kind; a connector that knows the role does. The size
  limits count only the files that are fetched.
- **A bundle in which the connector knows no role is refused**, exactly
  as an invalid bundle is: the connector keeps the version in use, or
  does not start if it has none. With no shapes the connector would
  check nothing and say nothing (there is no message check without
  `messages` shapes), a silent failure to the open side.
- **Everything else stays strict.** A field the format does not have is
  an error in the manifest, in a file entry and inside the four known
  roles. A role must be a JSON object with a `type` that is a string of
  1 to 64 letters, digits, `-` or `_`, starting with a letter (so every
  later role name is safe to log); anything else is an error. Only a
  well-formed role object with another type is skipped, and it may have
  any other fields. A file entry with such a role is still checked:
  path, digest, and a media type from the list (any of the three).
- **The issuer stays strict.** `portitor-issuer bundle build` refuses a
  role it does not know (it already did, with a less clear message),
  `sign_manifest` refuses to sign one, and the issuer's bundle route
  does not serve the files of a bundle whose manifest has one. An
  authority publishes a new role with the issuer version that knows it.
- The JSON Schema (`docs/schemas/bundle-manifest.schema.json`) says the
  same: the four known roles are closed, a role object with another
  type is valid, and at least one file must have a known role.

**The limit:** a connector that skips a new kind of rule does not
enforce it. An authority that needs every member to enforce a new role
must wait until they have upgraded (the WARN line and the gauge show
who has not), or make membership depend on the version.

Not changed: a media type outside the three listed still makes the
manifest invalid, so a later role that needs another media type cannot
be skipped by a 1.0 connector.

Where: `portitor_dcp::bundle` (`Role::Unknown`, read in two named steps
in `impl Deserialize for Role`; `Manifest::files_to_fetch`,
`check_files`), `portitor-bundle` (`source.rs`), the connector
(`shacl.rs`, `bundle_harvest.rs`), the issuer (`bundle_build.rs`,
`bundles.rs`).

## Spike result (2026-11-14)

Full report, with sources: [docs/spikes/0.6-shacl-engine.md](../spikes/0.6-shacl-engine.md).
The maintainer decides; this ADR stays Proposed.

Facts measured:
- **Correctness.** rudof, a small SHACL Core prototype of our own and the
  EU ITB validator give the same results on samples we wrote. A valid
  HealthDCAT-AP dataset passes. An invalid one gives exactly its 7 known
  violations, plus 1 from the vocabulary shapes. Caveat: the official
  release 7 host was blocked from the spike, so we used a third-party
  copy of release 7 `public-shapes.ttl` and `range.ttl`, and the release
  8 vocabulary shapes. Release 8 exists.
- **rudof `shacl` 0.3.39 fails `cargo deny check` with our `deny.toml`.**
  The reasons: MPL-2.0 and CDLA-Permissive-2.0 licences; reqwest is a
  fixed dependency of the `shacl` crate itself, and through it the banned
  base64 `simd-unsafe` feature comes in; two unmaintained crates; two
  quick-xml advisories, through `oxrdfxml`, with no fix. It adds 235
  crates (140 we don't have yet) and 4.3 MiB to a static musl binary. It
  builds on Rust 1.89 and runs without RocksDB.
- **An own subset on `oxrdf`, `oxttl` and `oxjsonld`** passes
  `cargo deny`, adds 22 crates (11 new) and 0.6 MiB, and builds on Rust
  1.89. The shapes use 9 constraint components (`nodeKind`, `minCount`,
  `node`, `class`, `hasValue`, `maxCount`, `datatype`, `or`, `pattern`),
  only `sh:targetClass`, and `sh:alternativePath`. Estimate: 1 to 2
  weeks with tests.
- **The ITB sidecar** (Jena and TopBraid SHACL, EUPL-1.2): a 787 MB image
  that uses about 284 MiB of memory when idle.
- **JSON-LD.** `oxjsonld` loads `@context` documents only through a
  callback we supply, and has no network code. A DSP 2025-1 dataset
  converts with the contexts taken from local files only.
- **Code lists dominate memory.** 453k triples take about 600 MiB in any
  engine; reduced to `skos:inScheme` and `rdf:type`, they take about
  60 MiB, with the same results. They are published as RDF/XML, whose
  Rust parser has the open advisories above.

Recommendation: implement Option 1 as **our own SHACL Core subset** on
Oxigraph's RDF crates (`oxrdf`, `oxttl`, `oxjsonld`), not rudof.
- The validator refuses shapes that use components it does not support.
- The ITB image is used only in tests, as a reference engine.
- The signed bundle carries code lists as reduced N-Triples.

Risks:
- bugs in our own engine, to be caught by the W3C test cases and by a
  comparison with ITB;
- new constraint components in later HealthDCAT-AP releases (release 8
  adds one SPARQL rule);
- rudof can be looked at again if it makes its HTTP and CLI dependencies
  optional.

Engine decided by the maintainer on 2026-11-14: **our own SHACL Core
subset**, as recommended above. The rest of this ADR (bundle format and
distribution) stays Proposed until 0.7.

## Consequences

- Validation adds latency per message. It is acceptable for catalog
  and negotiation messages (few per contract), not on the data path,
  which is not a DSP message exchange.
- 0.2 to 0.5 keep dataset metadata as JSON-LD and don't interpret it.
  `ldsc_dsp::Metadata` already does this (0.2).
- A profile such as HealthDCAT-AP is served with full IRIs for its terms
  under the plain DSP `@context` (decided 2026-11-14, see "Bundle
  format"); prefix definitions in the catalog `@context` are not used.
- A JSON-LD processor and possibly an RDF library become dependencies.
  Each needs its own justification and a licence check in the 0.6 pull
  requests.
- The manifest format gets its own JSON Schema and a fuzz target.
- Revisit this ADR if Gaia-X or Simpl-Open publish a standard way to
  distribute shapes; we should then follow it.
