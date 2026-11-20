// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The signed schema and vocabulary bundle of a dataspace authority
//! (ADR 0005, "Bundle format"). Pure code: no file or network access.
//!
//! A bundle is a folder with the files, `manifest.json` and
//! `manifest.jws`:
//!
//! - [`Manifest`] is `manifest.json`: the bundle's version, issue date,
//!   the authority's DID, the profile IRI (used as `dct:conformsTo`), and
//!   one [`FileEntry`] per file (path, SHA-256, media type, [`Role`]).
//!   The JSON Schema is `docs/schemas/bundle-manifest.schema.json`.
//! - `manifest.jws` is a compact JWS, `alg: ES256`, made by
//!   [`sign_manifest`] with the authority's **bundle key** (its own
//!   verification method in the authority's DID document, separate from
//!   the credential signing key).
//!
//! The JWS is *attached*: its payload is the exact bytes of
//! `manifest.json`. Why not a detached JWS (RFC 7515, appendix F): the
//! attached form is what [`crate::Jws`] already reads and checks, so no
//! second signature path is needed, and a reader that has only the JWS
//! still has the manifest. [`verify`] also checks that `manifest.json`
//! is byte for byte the signed payload, so the readable file can never
//! say something other than what was signed. The cost is a manifest
//! stored twice, a few kilobytes.
//!
//! Only the bundle key may sign a bundle: the `kid` must be
//! `<authority>#bundle-key` ([`BUNDLE_KEY_FRAGMENT`]) and the `alg`
//! ES256. The credential key, also listed under `assertionMethod`, is
//! refused, so a stolen credential key cannot sign a bundle.
//!
//! To use a bundle: [`signing_kid`] names the key that signed it,
//! [`key_from_document`] finds that key in the authority's DID document,
//! [`verify`] checks the signature and the manifest, and [`check_files`]
//! checks every file's digest and gives a [`VerifiedBundle`].

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::{Alg, DidDocument, Error, Jwk, Jws, Purpose, SigningKey};

/// The file name of the manifest in a bundle folder.
pub const MANIFEST_FILE: &str = "manifest.json";

/// The file name of the manifest's signature in a bundle folder.
pub const SIGNATURE_FILE: &str = "manifest.jws";

/// The file at the root of a folder of bundles that names the newest
/// version: `{"version": "2026.11.0"}` ([`Latest`]). It is not signed: a
/// connector only uses the version it names to find a manifest, and that
/// manifest must be signed and must not be older than the one in use.
pub const LATEST_FILE: &str = "latest.json";

/// The largest `latest.json` read, in bytes.
pub const MAX_LATEST_BYTES: usize = 4 * 1024;

/// The setting value that means "follow the newest version"
/// (`PORTITOR_BUNDLE_VERSION=latest`). No bundle may have it as its
/// version.
pub const FOLLOW_LATEST: &str = "latest";

/// The `typ` header of a bundle signature (RFC 8725, section 3.11:
/// explicit typing), so a signed credential or token can never pass as a
/// bundle signature, nor the other way round.
pub const JWS_TYPE: &str = "portitor-bundle+jws";

/// The fragment of the bundle key's id in the issuer's DID document
/// (`<authority DID>#bundle-key`).
pub const BUNDLE_KEY_FRAGMENT: &str = "bundle-key";

/// The most files one manifest may list.
pub const MAX_FILES: usize = 1_000;

/// The longest file path in a manifest, in bytes.
pub const MAX_PATH_LEN: usize = 200;

/// The longest bundle version, in bytes.
pub const MAX_VERSION_LEN: usize = 64;

/// The largest `manifest.json` read, in bytes (1 MiB).
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

/// The largest `manifest.jws` read, in bytes: twice
/// [`MAX_MANIFEST_BYTES`], because the JWS holds the manifest in base64
/// (4 characters for every 3 bytes) plus a header and a signature.
pub const MAX_SIGNATURE_BYTES: usize = MAX_MANIFEST_BYTES * 2;

/// The largest file of a bundle, in bytes (64 MiB). The build tool
/// refuses larger files, and a connector does not download them. Code
/// lists reduced to N-Triples are well below it (ADR 0005, spike).
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;

/// The largest bundle, all files together, in bytes (256 MiB).
pub const MAX_BUNDLE_BYTES: usize = 256 * 1024 * 1024;

/// Why a bundle cannot be built, signed or used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BundleError {
    /// `manifest.json` is not valid: the text says why.
    #[error("invalid manifest: {0}")]
    Manifest(String),
    /// A file path in the manifest is not allowed.
    #[error("invalid file path {path:?}: {reason}")]
    Path {
        /// The path as written.
        path: String,
        /// Why it is refused.
        reason: String,
    },
    /// The JWS cannot be read, or its signature does not verify.
    #[error("bundle signature: {0}")]
    Signature(#[from] Error),
    /// The JWS is signed with another algorithm than ES256 (a bundle
    /// key is a P-256 key, ADR 0005).
    #[error("bundle signature: alg is {0:?}, only ES256 is allowed")]
    WrongAlgorithm(Alg),
    /// The JWS has another `typ` than [`JWS_TYPE`].
    #[error("bundle signature: typ is not {JWS_TYPE}")]
    WrongType,
    /// The JWS has no `kid`, or a `kid` that is not the bundle key of the
    /// expected authority (`<authority>#bundle-key`).
    #[error(
        "bundle signature: key {kid:?} is not the bundle key {authority}#{BUNDLE_KEY_FRAGMENT}"
    )]
    WrongSigner {
        /// The `kid` of the JWS (empty if there is none).
        kid: String,
        /// The authority the bundle must come from.
        authority: String,
    },
    /// The key is not in the authority's DID document, or not listed
    /// under `assertionMethod`.
    #[error("bundle key {kid}: {reason}")]
    KeyNotInDocument {
        /// The key id.
        kid: String,
        /// Why the document does not give it.
        reason: String,
    },
    /// `manifest.json` is not the payload of `manifest.jws`.
    #[error("manifest.json is not the signed manifest")]
    NotTheSignedManifest,
    /// The manifest names another authority than the expected one.
    #[error("bundle authority is {found}, expected {expected}")]
    WrongAuthority {
        /// The expected authority DID.
        expected: String,
        /// The authority in the manifest.
        found: String,
    },
    /// The manifest's version is not the pinned one.
    #[error("bundle version is {found:?}, pinned {expected:?}")]
    WrongVersion {
        /// The pinned version.
        expected: String,
        /// The version in the manifest.
        found: String,
    },
    /// Every file of the manifest has a role this version does not
    /// know, so the bundle would give no rules at all.
    #[error(
        "invalid manifest: no file has a role this version knows \
         (datasets, messages, vocabulary, context)"
    )]
    NoKnownRole,
    /// A file the manifest lists was not given.
    #[error("bundle file {0} is missing")]
    MissingFile(String),
    /// A file's SHA-256 is not the one in the manifest.
    #[error("bundle file {0} does not match its SHA-256 digest")]
    DigestMismatch(String),
    /// A file was given that the manifest does not list.
    #[error("bundle file {0} is not listed in the manifest")]
    UnlistedFile(String),
}

/// Whether `issued` has the one form a manifest may use: an
/// `xsd:dateTime` in UTC, to the second, without fractions
/// (`YYYY-MM-DDThh:mm:ssZ`), one of the forms W3C DCAT 3 allows for
/// `dcterms:issued`. With one fixed form, comparing the text compares
/// the times, so versions issued on the same day keep their order.
#[must_use]
pub fn issued_is_valid(issued: &str) -> bool {
    issued.len() == 20
        && issued.as_bytes()[10] == b'T'
        && issued.ends_with('Z')
        && portitor_time::parse(issued).is_some()
}

/// `manifest.json`: what a bundle holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    /// The bundle version, for example `2026.11.0`. Also the name of the
    /// bundle's folder, so only letters, digits, `.`, `_` and `-`.
    pub version: String,
    /// When it was issued (`dcterms:issued` in W3C DCAT 3): an
    /// `xsd:dateTime` in UTC (`YYYY-MM-DDThh:mm:ssZ`). See
    /// [`issued_is_valid`].
    pub issued: String,
    /// The authority's `did:web` DID.
    pub authority: String,
    /// The profile IRI that a dataset passing these shapes states as its
    /// `dct:conformsTo`.
    pub profile: String,
    /// The files, at least one.
    pub files: Vec<FileEntry>,
}

/// One file of a bundle.
///
/// Read in two steps, as [`Role`] is: first as the manifest writes it
/// ([`FileEntryAsWritten`], the media type still a text), then the media
/// type is read by the rule of the file's role ([`media_type_for`]).
/// `try_from` tells serde to do exactly that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    try_from = "FileEntryAsWritten"
)]
pub struct FileEntry {
    /// The path inside the bundle folder, with `/` between folders, for
    /// example `shapes/public.ttl`. See [`check_path`] for the rules.
    pub path: String,
    /// The SHA-256 of the file's bytes, 64 lowercase hex digits.
    pub sha256: String,
    /// The file's media type.
    pub media_type: MediaType,
    /// What the file is for.
    pub role: Role,
}

/// A file entry exactly as JSON writes it: step 1 of reading a
/// [`FileEntry`]. Strict: a field the format does not have is an error.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileEntryAsWritten {
    path: String,
    sha256: String,
    media_type: String,
    role: Role,
}

// Step 2 of reading a `FileEntry`: the media type, by the role's rule.
impl TryFrom<FileEntryAsWritten> for FileEntry {
    type Error = String;

    fn try_from(written: FileEntryAsWritten) -> Result<Self, String> {
        let media_type = media_type_for(&written.role, &written.media_type)
            .map_err(|reason| format!("{}: mediaType: {reason}", written.path))?;
        Ok(Self {
            path: written.path,
            sha256: written.sha256,
            media_type,
            role: written.role,
        })
    }
}

/// The media type `text` of a file with the role `role`.
///
/// - A known role: one of the three media types this version reads,
///   nothing else.
/// - A role this version does not know ([`Role::Unknown`]): any text
///   with the shape of a media type ([`is_media_type_shape`]). The file
///   is skipped, so its media type is not used; a later role may need a
///   type outside today's three (for example `application/json`), and
///   that must not make a 1.0 connector refuse the whole bundle (ADR
///   0005, amended in 0.13).
///
/// # Errors
/// What is wrong with `text`.
fn media_type_for(role: &Role, text: &str) -> Result<MediaType, String> {
    if let Some(known) = MediaType::from_text(text) {
        return Ok(known);
    }
    if role.is_known() {
        return Err(format!(
            "{text:?} is not text/turtle, application/n-triples or application/ld+json"
        ));
    }
    if is_media_type_shape(text) {
        Ok(MediaType::Other)
    } else {
        Err("not a media type (type/subtype, RFC 6838 section 4.2)".to_owned())
    }
}

/// Whether `text` has the shape of a media type without parameters:
/// `type/subtype`, each a restricted name of RFC 6838 section 4.2.
#[must_use]
pub fn is_media_type_shape(text: &str) -> bool {
    match text.split_once('/') {
        Some((type_name, subtype_name)) => {
            is_restricted_name(type_name) && is_restricted_name(subtype_name)
        }
        None => false,
    }
}

/// RFC 6838 section 4.2, `restricted-name`: 1 to 127 characters, the
/// first a letter or a digit, the others letters, digits or one of
/// `! # $ & - ^ _ . +`. So no space, no `/`, no `;` and nothing that is
/// not ASCII: safe to log.
fn is_restricted_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    name.len() <= 127
        && first.is_ascii_alphanumeric()
        && characters.all(|c| c.is_ascii_alphanumeric() || "!#$&-^_.+".contains(c))
}

/// The media types a bundle file may have. RDF/XML is not one of them
/// (ADR 0005: no Portitor binary parses RDF/XML).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaType {
    /// Turtle.
    #[serde(rename = "text/turtle")]
    Turtle,
    /// N-Triples.
    #[serde(rename = "application/n-triples")]
    NTriples,
    /// JSON-LD (here only for JSON-LD contexts).
    #[serde(rename = "application/ld+json")]
    JsonLd,
    /// A media type this version does not read. Only a file whose role
    /// is [`Role::Unknown`] has it, and such a file is skipped, so the
    /// text is not kept. Never written and never read by this name: the
    /// two `skip` attributes make serde refuse both.
    #[serde(skip_serializing, skip_deserializing)]
    Other,
}

impl MediaType {
    /// The media type as written in HTTP and in the manifest.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Turtle => "text/turtle",
            Self::NTriples => "application/n-triples",
            Self::JsonLd => "application/ld+json",
            // the real text was not kept; this is HTTP's "some bytes"
            Self::Other => "application/octet-stream",
        }
    }

    /// The media type written as `text`, if it is one of the three this
    /// version reads (exactly as [`MediaType::as_str`] writes them).
    #[must_use]
    pub fn from_text(text: &str) -> Option<Self> {
        [Self::Turtle, Self::NTriples, Self::JsonLd]
            .into_iter()
            .find(|known| known.as_str() == text)
    }
}

/// What a bundle file is for. In JSON an object with a `type`, for
/// example `{"type": "datasets", "accessRights": ["…/PUBLIC"]}`.
///
/// The four known roles are read strictly: a field they do not have is
/// an error. A role object whose `type` this version does not know is
/// read as [`Role::Unknown`], whatever other fields it has: a later
/// version of the format may add roles, and a reader of this version
/// skips such a file (ADR 0005, amended in 0.13). How it is read is in
/// `impl Deserialize for Role` below.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// SHACL shapes for dataset metadata, for the datasets whose
    /// `dct:accessRights` is one of these IRIs (for example
    /// `http://publications.europa.eu/resource/authority/access-right/PUBLIC`).
    Datasets {
        /// The `dct:accessRights` IRIs the shapes apply to, at least one.
        access_rights: Vec<String>,
    },
    /// SHACL shapes for DSP messages.
    Messages {},
    /// Ontologies and code lists, as data for `sh:class` checks.
    Vocabulary {},
    /// A JSON-LD context, standing for the context at `url`. The
    /// validator uses it instead of fetching `url`.
    Context {
        /// The context URL it stands for.
        url: String,
    },
    /// A role this version does not know. The file is skipped: not
    /// fetched, not checked, not used. Never written: the build tool
    /// refuses it.
    Unknown {
        /// The role's `type` as the manifest writes it.
        type_name: String,
    },
}

impl Role {
    /// Whether this version knows the role, so that it uses the file.
    #[must_use]
    pub fn is_known(&self) -> bool {
        !matches!(self, Role::Unknown { .. })
    }
}

/// The longest `type` of a role, in bytes.
pub const MAX_ROLE_TYPE_LEN: usize = 64;

/// Whether `type_name` may be the `type` of a role: 1 to
/// [`MAX_ROLE_TYPE_LEN`] ASCII letters, digits, `-` or `_`, starting
/// with a letter. Every role name of a later version must fit; the rule
/// also keeps an unknown name safe to write into a log line.
#[must_use]
pub fn is_role_type_name(type_name: &str) -> bool {
    type_name.len() <= MAX_ROLE_TYPE_LEN
        && type_name.starts_with(|first: char| first.is_ascii_alphabetic())
        && type_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The known roles exactly as JSON writes them. serde's derive reads and
/// writes this enum; [`Role`] is the same plus [`Role::Unknown`].
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum KnownRole {
    #[serde(rename_all = "camelCase")]
    Datasets {
        access_rights: Vec<String>,
    },
    // Written `Messages {}` rather than `Messages`: serde refuses unknown
    // fields only in variants with braces.
    Messages {},
    Vocabulary {},
    Context {
        url: String,
    },
}

/// The `type` values of [`KnownRole`].
const KNOWN_ROLE_TYPES: [&str; 4] = ["datasets", "messages", "vocabulary", "context"];

impl From<KnownRole> for Role {
    fn from(known: KnownRole) -> Self {
        match known {
            KnownRole::Datasets { access_rights } => Role::Datasets { access_rights },
            KnownRole::Messages {} => Role::Messages {},
            KnownRole::Vocabulary {} => Role::Vocabulary {},
            KnownRole::Context { url } => Role::Context { url },
        }
    }
}

/// Only the `type` of a role object; every other field is passed over.
#[derive(Deserialize)]
struct RoleTypeOnly {
    #[serde(rename = "type")]
    type_name: String,
}

// Reading a role in two steps (docs/RUST-IDIOMS.md, "Reading JSON in two
// steps"). serde's derive cannot say "strict for the names I know, open
// for the others", so the rule is written out here:
//
// 1. Keep the role's JSON text as it is (`RawValue`) and read only its
//    `type` from it. A role that is not an object, or has no `type`
//    that is a string, is an error.
// 2. A known `type`: read the same text again as `KnownRole`, strictly
//    (an unknown field is an error). Any other `type`: `Role::Unknown`,
//    and the other fields are not looked at.
impl<'de> Deserialize<'de> for Role {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;

        let text = Box::<RawValue>::deserialize(deserializer)?;
        // serde would also read `RoleTypeOnly` from a JSON array
        // (`["messages"]`), so the object is asked for here
        if !text.get().starts_with('{') {
            return Err(D::Error::custom("role: not a JSON object"));
        }
        let RoleTypeOnly { type_name } =
            serde_json::from_str(text.get()).map_err(|e| D::Error::custom(format!("role: {e}")))?;
        if KNOWN_ROLE_TYPES.contains(&type_name.as_str()) {
            let known: KnownRole = serde_json::from_str(text.get())
                .map_err(|e| D::Error::custom(format!("role {type_name}: {e}")))?;
            return Ok(known.into());
        }
        // `Messages` for `messages` is a spelling mistake, not a role of a
        // later version: skipping it would leave its rules unchecked
        // with only a warning
        if let Some(known) = KNOWN_ROLE_TYPES
            .iter()
            .find(|known| known.eq_ignore_ascii_case(&type_name))
        {
            return Err(D::Error::custom(format!(
                "role: type {type_name} is not known; the type is written {known}"
            )));
        }
        if !is_role_type_name(&type_name) {
            return Err(D::Error::custom(format!(
                "role: type must be 1 to {MAX_ROLE_TYPE_LEN} letters, digits, '-' or '_', \
                 starting with a letter"
            )));
        }
        Ok(Role::Unknown { type_name })
    }
}

// Writing: the known roles as `KnownRole` writes them. An unknown role
// cannot be written, because its other fields were not kept; nothing
// writes one (the build tool refuses it before).
impl Serialize for Role {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let known = match self.clone() {
            Role::Datasets { access_rights } => KnownRole::Datasets { access_rights },
            Role::Messages {} => KnownRole::Messages {},
            Role::Vocabulary {} => KnownRole::Vocabulary {},
            Role::Context { url } => KnownRole::Context { url },
            Role::Unknown { type_name } => {
                return Err(serde::ser::Error::custom(format!(
                    "a file with the unknown role {type_name} cannot be written"
                )));
            }
        };
        known.serialize(serializer)
    }
}

impl Manifest {
    /// Reads `manifest.json` strictly (unknown or repeated fields are
    /// refused) and checks it with [`Manifest::check`].
    ///
    /// # Errors
    /// Not such a manifest.
    pub fn parse(bytes: &[u8]) -> Result<Self, BundleError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(BundleError::Manifest(format!(
                "larger than {MAX_MANIFEST_BYTES} bytes"
            )));
        }
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|e| BundleError::Manifest(e.to_string()))?;
        manifest.check()?;
        Ok(manifest)
    }

    /// Checks the rules JSON alone cannot express: a safe version, a
    /// date, a `did:web` authority, IRIs, at least one and at most
    /// [`MAX_FILES`] files with distinct safe paths, hex digests, media
    /// types that fit the role, distinct context URLs, and at least one
    /// file with a role this version knows.
    ///
    /// # Errors
    /// The first rule broken.
    pub fn check(&self) -> Result<(), BundleError> {
        check_version(&self.version)?;
        if !issued_is_valid(&self.issued) {
            return Err(manifest_error(
                "issued is not a UTC date and time (YYYY-MM-DDThh:mm:ssZ)",
            ));
        }
        crate::did_web_url(&self.authority)
            .map_err(|e| BundleError::Manifest(format!("authority: {e}")))?;
        if !is_iri(&self.profile) {
            return Err(manifest_error("profile is not an http(s) IRI"));
        }
        if self.files.is_empty() {
            return Err(manifest_error("no files"));
        }
        if self.files.len() > MAX_FILES {
            return Err(BundleError::Manifest(format!(
                "more than {MAX_FILES} files"
            )));
        }
        let mut paths = BTreeSet::new();
        let mut context_urls = BTreeSet::new();
        for entry in &self.files {
            check_path(&entry.path)?;
            // two paths that differ only in case would be one file on
            // some file systems
            if !paths.insert(entry.path.to_ascii_lowercase()) {
                return Err(BundleError::Manifest(format!(
                    "{} is listed twice",
                    entry.path
                )));
            }
            check_entry(entry)?;
            if let Role::Context { url } = &entry.role
                && !context_urls.insert(url.as_str())
            {
                return Err(BundleError::Manifest(format!(
                    "two files stand for the context {url}"
                )));
            }
        }
        // A bundle of which nothing is understood gives no rules at all.
        // Using it would mean checking nothing without saying so, so it
        // is refused like any other invalid bundle.
        if !self.files.iter().any(|entry| entry.role.is_known()) {
            return Err(BundleError::NoKnownRole);
        }
        Ok(())
    }

    /// The files whose role this version does not know, in manifest
    /// order. A reader skips them.
    #[must_use]
    pub fn files_of_unknown_role(&self) -> Vec<SkippedFile<'_>> {
        let mut skipped = Vec::new();
        for entry in &self.files {
            if let Role::Unknown { type_name } = &entry.role {
                skipped.push(SkippedFile {
                    path: &entry.path,
                    role_type: type_name,
                });
            }
        }
        skipped
    }

    /// The files a reader fetches and checks: those with a role this
    /// version knows, in manifest order.
    #[must_use]
    pub fn files_to_fetch(&self) -> Vec<&FileEntry> {
        self.files
            .iter()
            .filter(|entry| entry.role.is_known())
            .collect()
    }

    /// The entry of the file at `path`, if the manifest lists it.
    #[must_use]
    pub fn entry(&self, path: &str) -> Option<&FileEntry> {
        self.files.iter().find(|entry| entry.path == path)
    }
}

/// A file a reader skips because it does not know the file's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkippedFile<'a> {
    /// The file's path in the bundle.
    pub path: &'a str,
    /// The `type` of its role.
    pub role_type: &'a str,
}

/// A manifest error with a fixed text.
fn manifest_error(reason: &str) -> BundleError {
    BundleError::Manifest(reason.to_owned())
}

/// Whether `text` looks like an absolute `http`/`https` IRI.
fn is_iri(text: &str) -> bool {
    let rest = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"));
    rest.is_some_and(|rest| !rest.is_empty() && !rest.contains(char::is_whitespace))
}

/// Whether `c` may appear in a version or a path segment.
fn is_safe_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// A version is also a folder and URL segment: 1 to [`MAX_VERSION_LEN`]
/// letters, digits,
/// `.`, `_` or `-`, not starting with `.`.
///
/// # Errors
/// The version breaks one of these rules.
pub fn check_version(version: &str) -> Result<(), BundleError> {
    let fits = !version.is_empty()
        && version.len() <= MAX_VERSION_LEN
        && !version.starts_with('.')
        && version.chars().all(is_safe_char);
    if !fits {
        return Err(BundleError::Manifest(format!(
            "version must be 1 to {MAX_VERSION_LEN} letters, digits, '.', '_' or '-', \
             not starting with '.'"
        )));
    }
    if version.eq_ignore_ascii_case(FOLLOW_LATEST) {
        return Err(BundleError::Manifest(format!(
            "version must not be {FOLLOW_LATEST:?}: that word means \"the newest version\" in a connector's settings"
        )));
    }
    Ok(())
}

/// `latest.json` ([`LATEST_FILE`]): the newest version in a folder of
/// bundles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Latest {
    /// The newest version; the name of its folder.
    pub version: String,
}

impl Latest {
    /// Reads `latest.json`.
    ///
    /// # Errors
    /// Not this JSON object, or a version that [`check_version`] refuses.
    pub fn parse(bytes: &[u8]) -> Result<Self, BundleError> {
        let latest: Self = serde_json::from_slice(bytes)
            .map_err(|e| BundleError::Manifest(format!("{LATEST_FILE}: {e}")))?;
        check_version(&latest.version)
            .map_err(|e| BundleError::Manifest(format!("{LATEST_FILE}: {e}")))?;
        Ok(latest)
    }
}

/// A file path must stay inside the bundle folder and mean the same on
/// every system: relative, `/` between folders, each part made of
/// letters, digits, `.`, `_` and `-` and not starting with `.` (so no
/// `..`, no hidden files), at most [`MAX_PATH_LEN`] bytes, and not
/// `manifest.json` or `manifest.jws`.
///
/// # Errors
/// The path breaks one of these rules.
pub fn check_path(path: &str) -> Result<(), BundleError> {
    let refuse = |reason: &str| {
        Err(BundleError::Path {
            path: path.to_owned(),
            reason: reason.to_owned(),
        })
    };
    if path.is_empty() || path.len() > MAX_PATH_LEN {
        return refuse(&format!("empty or longer than {MAX_PATH_LEN} bytes"));
    }
    if path.starts_with('/') {
        return refuse("absolute");
    }
    if path.contains('\\') {
        return refuse("backslash");
    }
    for part in path.split('/') {
        if part.is_empty() {
            return refuse("empty folder name");
        }
        if part.starts_with('.') {
            // also refuses "." and ".."
            return refuse("a part starts with '.'");
        }
        if !part.chars().all(is_safe_char) {
            return refuse("only letters, digits, '.', '_' and '-' are allowed");
        }
    }
    if path == MANIFEST_FILE || path == SIGNATURE_FILE {
        return refuse("reserved name");
    }
    Ok(())
}

/// The rules of one entry besides its path.
fn check_entry(entry: &FileEntry) -> Result<(), BundleError> {
    let wrong = |reason: &str| Err(BundleError::Manifest(format!("{}: {reason}", entry.path)));
    let is_hex = entry.sha256.len() == 64
        && entry
            .sha256
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    if !is_hex {
        return wrong("sha256 is not 64 lowercase hex digits");
    }
    let is_context = matches!(entry.role, Role::Context { .. });
    let is_json_ld = entry.media_type == MediaType::JsonLd;
    // which media type fits an unknown role is not known either
    if entry.role.is_known() && is_context != is_json_ld {
        return wrong("a context is application/ld+json, and every other file is RDF");
    }
    match &entry.role {
        Role::Datasets { access_rights } => {
            if access_rights.is_empty() {
                return wrong("datasets shapes name no accessRights");
            }
            if !access_rights.iter().all(|iri| is_iri(iri)) {
                return wrong("an accessRights value is not an http(s) IRI");
            }
        }
        Role::Context { url } => {
            if !is_iri(url) {
                return wrong("the context url is not an http(s) URL");
            }
        }
        Role::Messages {} | Role::Vocabulary {} | Role::Unknown { .. } => {}
    }
    Ok(())
}

/// The SHA-256 of `bytes`, as 64 lowercase hex digits (the form the
/// manifest uses). The one SHA-256 helper of the workspace: other crates
/// call this one rather than writing their own.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest.as_ref() {
        // writing to a String cannot fail
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Signs `manifest_bytes` (the exact bytes of `manifest.json`) as a
/// compact JWS with `alg: ES256`, `typ:` [`JWS_TYPE`] and `kid`, the DID
/// URL of the authority's bundle key, `<authority>#bundle-key` (see
/// [`bundle_kid`]). The payload is the bytes as
/// they are, so `manifest.json` must have no white space before or after
/// the JSON (a JSON writer adds none).
///
/// # Errors
/// The manifest is not valid ([`Manifest::parse`]), lists a file with a
/// role this version does not know ([`Role::Unknown`]), has white space
/// around it, `kid` is not the bundle key of the manifest's authority, or signing
/// fails.
pub fn sign_manifest(
    manifest_bytes: &[u8],
    key: &SigningKey,
    kid: &str,
) -> Result<String, BundleError> {
    let manifest = Manifest::parse(manifest_bytes)?;
    // a reader skips a role it does not know; the signer is strict: it
    // never publishes a role this version does not know
    if let Some(unknown) = manifest.files_of_unknown_role().first() {
        return Err(BundleError::Manifest(format!(
            "{}: the role {} is not known to this version and cannot be signed",
            unknown.path, unknown.role_type
        )));
    }
    check_kid(kid, &manifest.authority)?;
    let text = std::str::from_utf8(manifest_bytes)
        .map_err(|_| manifest_error("not UTF-8"))?
        .to_owned();
    if text.trim() != text {
        return Err(manifest_error("white space before or after the JSON"));
    }
    // A RawValue is written out exactly as it was read, so the JWS
    // payload is the manifest's bytes, not a re-written copy.
    let raw = RawValue::from_string(text).map_err(|e| BundleError::Manifest(e.to_string()))?;
    Ok(key.sign_json(kid, JWS_TYPE, &raw)?)
}

/// What a bundle must be: from this authority, at this version.
#[derive(Debug, Clone, Copy)]
pub struct Expected<'a> {
    /// The authority's DID (the connector's `PORTITOR_BUNDLE_AUTHORITY`).
    pub authority: &'a str,
    /// The version asked for: the connector's `PORTITOR_BUNDLE_VERSION`,
    /// or the one `latest.json` named.
    pub version: &'a str,
}

/// The `kid` of a bundle signature, read before it is verified, so the
/// caller knows which key to look up. It must be the bundle key of
/// `authority` (`<authority>#bundle-key`), and the `alg` ES256.
///
/// # Errors
/// The JWS cannot be read, is not ES256, or its `kid` is not the bundle
/// key of `authority`.
pub fn signing_kid(jws: &str, authority: &str) -> Result<String, BundleError> {
    let parsed = Jws::parse(jws)?;
    check_alg(&parsed)?;
    let kid = parsed.kid().unwrap_or_default();
    check_kid(kid, authority)?;
    Ok(kid.to_owned())
}

/// The id of the bundle key of `authority`: `<authority>#bundle-key`.
#[must_use]
pub fn bundle_kid(authority: &str) -> String {
    format!("{authority}#{BUNDLE_KEY_FRAGMENT}")
}

/// `kid` is the bundle key of `authority`, `<authority>#bundle-key`.
/// Any other key of the authority, such as its credential key, is
/// refused (ADR 0005: only the bundle key signs a bundle).
fn check_kid(kid: &str, authority: &str) -> Result<(), BundleError> {
    if kid == bundle_kid(authority) {
        Ok(())
    } else {
        Err(BundleError::WrongSigner {
            kid: kid.to_owned(),
            authority: authority.to_owned(),
        })
    }
}

/// The JWS is signed with ES256. [`Jws::parse`] also accepts PS256 (for
/// credentials), which a bundle key never uses.
fn check_alg(parsed: &Jws<'_>) -> Result<(), BundleError> {
    match parsed.alg() {
        Alg::ES256 => Ok(()),
        Alg::PS256 => Err(BundleError::WrongAlgorithm(Alg::PS256)),
    }
}

/// The public key `kid` in the authority's DID document. The document
/// must be the authority's, and the key must be listed under
/// `assertionMethod` (the authority asserts the bundle's content).
///
/// # Errors
/// Another DID's document, `kid` not of the authority, not in the
/// document, or not under `assertionMethod`.
pub fn key_from_document<'d>(
    document: &'d DidDocument,
    kid: &str,
    authority: &str,
) -> Result<&'d Jwk, BundleError> {
    check_kid(kid, authority)?;
    let not_found = |reason: String| BundleError::KeyNotInDocument {
        kid: kid.to_owned(),
        reason,
    };
    if document.id != authority {
        return Err(not_found(format!(
            "the DID document is for {}",
            document.id
        )));
    }
    let method = document
        .key_for(Some(kid), Purpose::AssertionMethod)
        .map_err(|e| not_found(e.to_string()))?;
    Ok(&method.public_key_jwk)
}

/// Checks a bundle's signature and manifest: `jws` is an ES256
/// [`JWS_TYPE`] JWS by the bundle key of `expected.authority`
/// (`<authority>#bundle-key`), verified with `key` (found with
/// [`key_from_document`], or kept from an earlier start); its payload is
/// exactly `manifest_bytes`; the manifest is valid, names that authority
/// and has the pinned version.
///
/// # Errors
/// The first check that fails.
pub fn verify(
    manifest_bytes: &[u8],
    jws: &str,
    key: &Jwk,
    expected: Expected<'_>,
) -> Result<Manifest, BundleError> {
    if jws.len() > MAX_SIGNATURE_BYTES {
        return Err(BundleError::Signature(Error::Malformed("too large")));
    }
    let parsed = Jws::parse(jws)?;
    if parsed.typ() != Some(JWS_TYPE) {
        return Err(BundleError::WrongType);
    }
    check_alg(&parsed)?;
    check_kid(parsed.kid().unwrap_or_default(), expected.authority)?;
    parsed.verify(key)?;
    if parsed.payload() != manifest_bytes {
        return Err(BundleError::NotTheSignedManifest);
    }
    let manifest = Manifest::parse(manifest_bytes)?;
    if manifest.authority != expected.authority {
        return Err(BundleError::WrongAuthority {
            expected: expected.authority.to_owned(),
            found: manifest.authority,
        });
    }
    if manifest.version != expected.version {
        return Err(BundleError::WrongVersion {
            expected: expected.version.to_owned(),
            found: manifest.version,
        });
    }
    Ok(manifest)
}

/// Checks the files of a verified manifest: every listed file with a
/// known role is in `files` (path to bytes) and has the listed SHA-256.
/// A file in `files` that the manifest does not list is refused
/// ([`BundleError::UnlistedFile`]): only what the authority signed may
/// reach the validator. (Adapters read only the listed files, so other
/// files in a bundle folder are never read.)
///
/// A file whose role this version does not know ([`Role::Unknown`]) is
/// skipped: it need not be in `files`, and if it is, its bytes are
/// dropped unread. Adapters do not fetch such a file
/// ([`Manifest::files_to_fetch`]). This is safe because nothing uses
/// the bytes: the signature covers the manifest, so the path, the
/// digest and the role of the skipped file are still the authority's,
/// and [`VerifiedBundle::skipped_files`] reports them.
///
/// # Errors
/// A file is missing, does not match its digest, or is not listed.
pub fn check_files(
    manifest: Manifest,
    mut files: BTreeMap<String, Vec<u8>>,
) -> Result<VerifiedBundle, BundleError> {
    let mut checked = BTreeMap::new();
    for entry in &manifest.files {
        let given = files.remove(&entry.path);
        if !entry.role.is_known() {
            continue;
        }
        let Some(bytes) = given else {
            return Err(BundleError::MissingFile(entry.path.clone()));
        };
        if sha256_hex(&bytes) != entry.sha256 {
            return Err(BundleError::DigestMismatch(entry.path.clone()));
        }
        checked.insert(entry.path.clone(), bytes);
    }
    // what is left was not listed
    if let Some(unlisted) = files.into_keys().next() {
        return Err(BundleError::UnlistedFile(unlisted));
    }
    Ok(VerifiedBundle {
        manifest,
        files: checked,
    })
}

/// A bundle whose signature and digests were checked: the manifest and
/// the bytes of each file with a known role. Only [`check_files`] makes one, so holding a
/// `VerifiedBundle` means every file is what the authority signed.
///
/// The bytes are not parsed: Turtle and N-Triples for shapes and
/// vocabularies, JSON-LD for contexts. The SHACL engine reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBundle {
    manifest: Manifest,
    files: BTreeMap<String, Vec<u8>>,
}

/// One file of a [`VerifiedBundle`]: its manifest entry and its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundleFile<'a> {
    /// The manifest entry (path, digest, media type, role).
    pub entry: &'a FileEntry,
    /// The file's bytes.
    pub bytes: &'a [u8],
}

impl VerifiedBundle {
    /// The bundle version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.manifest.version
    }

    /// When it was issued (`YYYY-MM-DDThh:mm:ssZ`). A
    /// connector that follows the newest version never takes one issued
    /// before the one it uses; see [`issued_is_valid`] for the order.
    #[must_use]
    pub fn issued(&self) -> &str {
        &self.manifest.issued
    }

    /// The profile IRI, to state as `dct:conformsTo` of a dataset that
    /// passes the shapes.
    #[must_use]
    pub fn profile(&self) -> &str {
        &self.manifest.profile
    }

    /// The bytes of the file at `path`, if listed.
    #[must_use]
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    /// Every file that is used (every file with a known role), in
    /// manifest order.
    #[must_use]
    pub fn files(&self) -> Vec<BundleFile<'_>> {
        self.select(|_| true)
    }

    /// The files that are skipped because this version does not know
    /// their role, in manifest order. Whatever they hold is not enforced.
    #[must_use]
    pub fn skipped_files(&self) -> Vec<SkippedFile<'_>> {
        self.manifest.files_of_unknown_role()
    }

    /// The dataset shapes for datasets whose `dct:accessRights` is
    /// `access_right` (an IRI, compared exactly). Empty when the bundle
    /// has none for it.
    #[must_use]
    pub fn datasets_files_for(&self, access_right: &str) -> Vec<BundleFile<'_>> {
        self.select(|entry| match &entry.role {
            Role::Datasets { access_rights } => access_rights.iter().any(|a| a == access_right),
            _ => false,
        })
    }

    /// Every `dct:accessRights` IRI some dataset shapes apply to, sorted.
    #[must_use]
    pub fn access_rights(&self) -> BTreeSet<&str> {
        let mut found = BTreeSet::new();
        for entry in &self.manifest.files {
            if let Role::Datasets { access_rights } = &entry.role {
                found.extend(access_rights.iter().map(String::as_str));
            }
        }
        found
    }

    /// The shapes for DSP messages.
    #[must_use]
    pub fn messages_files(&self) -> Vec<BundleFile<'_>> {
        self.select(|entry| matches!(entry.role, Role::Messages {}))
    }

    /// The vocabularies (ontologies and code lists).
    #[must_use]
    pub fn vocabulary_files(&self) -> Vec<BundleFile<'_>> {
        self.select(|entry| matches!(entry.role, Role::Vocabulary {}))
    }

    /// The JSON-LD contexts, by the URL each stands for. A JSON-LD reader
    /// takes contexts from here only, never from the network (ADR 0005).
    #[must_use]
    pub fn contexts(&self) -> BTreeMap<&str, &[u8]> {
        let mut found = BTreeMap::new();
        for file in self.files() {
            if let Role::Context { url } = &file.entry.role {
                found.insert(url.as_str(), file.bytes);
            }
        }
        found
    }

    /// The files whose entry passes `keep`, in manifest order.
    fn select(&self, keep: impl Fn(&FileEntry) -> bool) -> Vec<BundleFile<'_>> {
        let mut selected = Vec::new();
        for entry in &self.manifest.files {
            if !keep(entry) {
                continue;
            }
            // check_files put every listed file with a known role in
            // `files`; a skipped file has no bytes and is left out
            if let Some(bytes) = self.files.get(&entry.path) {
                selected.push(BundleFile { entry, bytes });
            }
        }
        selected
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{Relationship, VerificationMethod};
    use base64::Engine as _;

    const AUTHORITY: &str = "did:web:authority.example";
    const PUBLIC: &str = "http://publications.europa.eu/resource/authority/access-right/PUBLIC";
    const RESTRICTED: &str =
        "http://publications.europa.eu/resource/authority/access-right/RESTRICTED";
    const SHAPES: &[u8] = b"@prefix sh: <http://www.w3.org/ns/shacl#> .\n";
    const CONTEXT: &[u8] = br#"{"@context": {"dct": "http://purl.org/dc/terms/"}}"#;

    fn kid() -> String {
        format!("{AUTHORITY}#{BUNDLE_KEY_FRAGMENT}")
    }

    fn manifest() -> Manifest {
        Manifest {
            version: "2026.11.0".to_owned(),
            issued: "2026-11-14T00:00:00Z".to_owned(),
            authority: AUTHORITY.to_owned(),
            profile: "https://healthdcat-ap.example/release-8".to_owned(),
            files: vec![
                FileEntry {
                    path: "shapes/public.ttl".to_owned(),
                    sha256: sha256_hex(SHAPES),
                    media_type: MediaType::Turtle,
                    role: Role::Datasets {
                        access_rights: vec![PUBLIC.to_owned()],
                    },
                },
                FileEntry {
                    path: "contexts/dct.jsonld".to_owned(),
                    sha256: sha256_hex(CONTEXT),
                    media_type: MediaType::JsonLd,
                    role: Role::Context {
                        url: "https://contexts.example/dct.jsonld".to_owned(),
                    },
                },
            ],
        }
    }

    fn files() -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([
            ("shapes/public.ttl".to_owned(), SHAPES.to_vec()),
            ("contexts/dct.jsonld".to_owned(), CONTEXT.to_vec()),
        ])
    }

    fn expected() -> Expected<'static> {
        Expected {
            authority: AUTHORITY,
            version: "2026.11.0",
        }
    }

    fn signed(key: &SigningKey) -> (Vec<u8>, String) {
        let bytes = serde_json::to_vec_pretty(&manifest()).unwrap();
        let jws = sign_manifest(&bytes, key, &kid()).unwrap();
        (bytes, jws)
    }

    fn document(key: &SigningKey, relationship: bool) -> DidDocument {
        DidDocument {
            id: AUTHORITY.to_owned(),
            verification_method: vec![VerificationMethod {
                id: kid(),
                kind: "JsonWebKey2020".to_owned(),
                controller: AUTHORITY.to_owned(),
                public_key_jwk: key.public_jwk(),
            }],
            assertion_method: if relationship {
                vec![Relationship::Reference(kid())]
            } else {
                Vec::new()
            },
            ..DidDocument::default()
        }
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let key = SigningKey::generate().unwrap();
        let (bytes, jws) = signed(&key);
        assert_eq!(signing_kid(&jws, AUTHORITY).unwrap(), kid());
        let doc = document(&key, true);
        let jwk = key_from_document(&doc, &kid(), AUTHORITY).unwrap();
        let checked = verify(&bytes, &jws, jwk, expected()).unwrap();
        assert_eq!(checked, manifest());
        let bundle = check_files(checked, files()).unwrap();
        assert_eq!(bundle.version(), "2026.11.0");
        assert_eq!(bundle.datasets_files_for(PUBLIC).len(), 1);
        assert!(bundle.datasets_files_for(RESTRICTED).is_empty());
        assert_eq!(
            bundle.access_rights().into_iter().collect::<Vec<_>>(),
            [PUBLIC]
        );
        assert_eq!(
            bundle.contexts().get("https://contexts.example/dct.jsonld"),
            Some(&CONTEXT)
        );
        assert!(bundle.messages_files().is_empty());
        assert_eq!(bundle.files().len(), 2);
        assert_eq!(bundle.file("shapes/public.ttl"), Some(SHAPES));
    }

    #[test]
    fn a_wrong_signature_is_refused() {
        let key = SigningKey::generate().unwrap();
        let other = SigningKey::generate().unwrap();
        let (bytes, jws) = signed(&key);
        assert_eq!(
            verify(&bytes, &jws, &other.public_jwk(), expected()),
            Err(BundleError::Signature(Error::BadSignature))
        );
        // a changed payload with the old signature
        let (header, rest) = jws.split_once('.').unwrap();
        let (_, signature) = rest.split_once('.').unwrap();
        let mut changed = manifest();
        changed.profile = "https://other.example/profile".to_owned();
        let changed_bytes = serde_json::to_vec(&changed).unwrap();
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&changed_bytes);
        let forged = format!("{header}.{payload}.{signature}");
        assert_eq!(
            verify(&changed_bytes, &forged, &key.public_jwk(), expected()),
            Err(BundleError::Signature(Error::BadSignature))
        );
    }

    #[test]
    fn manifest_json_must_be_the_signed_bytes() {
        let key = SigningKey::generate().unwrap();
        let (_, jws) = signed(&key);
        let rewritten = serde_json::to_vec(&manifest()).unwrap();
        assert_eq!(
            verify(&rewritten, &jws, &key.public_jwk(), expected()),
            Err(BundleError::NotTheSignedManifest)
        );
    }

    #[test]
    fn another_signer_is_refused() {
        let key = SigningKey::generate().unwrap();
        let mut other = manifest();
        other.authority = "did:web:other.example".to_owned();
        let bytes = serde_json::to_vec(&other).unwrap();
        let jws = sign_manifest(&bytes, &key, "did:web:other.example#bundle-key").unwrap();
        assert!(matches!(
            signing_kid(&jws, AUTHORITY),
            Err(BundleError::WrongSigner { .. })
        ));
        assert!(matches!(
            verify(&bytes, &jws, &key.public_jwk(), expected()),
            Err(BundleError::WrongSigner { .. })
        ));
        // the kid must be a key of the manifest's own authority
        assert!(matches!(
            sign_manifest(&bytes, &key, &kid()),
            Err(BundleError::WrongSigner { .. })
        ));
        // a manifest of another authority, signed with a key of ours
        let jws = key
            .sign_json(
                &kid(),
                JWS_TYPE,
                &RawValue::from_string(String::from_utf8(bytes.clone()).unwrap()).unwrap(),
            )
            .unwrap();
        assert!(matches!(
            verify(&bytes, &jws, &key.public_jwk(), expected()),
            Err(BundleError::WrongAuthority { .. })
        ));
    }

    #[test]
    fn the_key_must_be_an_assertion_method_of_the_authority() {
        let key = SigningKey::generate().unwrap();
        assert!(matches!(
            key_from_document(&document(&key, false), &kid(), AUTHORITY),
            Err(BundleError::KeyNotInDocument { .. })
        ));
        // a key of the authority, but not its bundle key
        assert!(matches!(
            key_from_document(
                &document(&key, true),
                &format!("{AUTHORITY}#other"),
                AUTHORITY
            ),
            Err(BundleError::WrongSigner { .. })
        ));
        let mut foreign = document(&key, true);
        foreign.id = "did:web:other.example".to_owned();
        assert!(matches!(
            key_from_document(&foreign, &kid(), AUTHORITY),
            Err(BundleError::KeyNotInDocument { .. })
        ));
    }

    /// The authority's DID document as the issuer publishes it: the
    /// credential key `#key-1` and the bundle key, both under
    /// `assertionMethod`.
    fn document_with_both_keys(credential_key: &Jwk, bundle_key: &Jwk) -> DidDocument {
        let credential_kid = format!("{AUTHORITY}#key-1");
        let method = |id: &str, jwk: &Jwk| VerificationMethod {
            id: id.to_owned(),
            kind: "JsonWebKey2020".to_owned(),
            controller: AUTHORITY.to_owned(),
            public_key_jwk: jwk.clone(),
        };
        DidDocument {
            id: AUTHORITY.to_owned(),
            verification_method: vec![
                method(&credential_kid, credential_key),
                method(&kid(), bundle_key),
            ],
            assertion_method: vec![
                Relationship::Reference(credential_kid),
                Relationship::Reference(kid()),
            ],
            ..DidDocument::default()
        }
    }

    #[test]
    fn a_bundle_signed_with_the_credential_key_is_refused() {
        let credential_key = SigningKey::generate().unwrap();
        let bundle_key = SigningKey::generate().unwrap();
        let doc = document_with_both_keys(&credential_key.public_jwk(), &bundle_key.public_jwk());
        let credential_kid = format!("{AUTHORITY}#key-1");
        // sign_manifest itself refuses another kid ...
        let bytes = serde_json::to_vec(&manifest()).unwrap();
        assert!(matches!(
            sign_manifest(&bytes, &credential_key, &credential_kid),
            Err(BundleError::WrongSigner { .. })
        ));
        // ... and a connector refuses such a JWS made by hand
        let raw = RawValue::from_string(String::from_utf8(bytes.clone()).unwrap()).unwrap();
        let jws = credential_key
            .sign_json(&credential_kid, JWS_TYPE, &raw)
            .unwrap();
        assert!(matches!(
            signing_kid(&jws, AUTHORITY),
            Err(BundleError::WrongSigner { .. })
        ));
        assert!(matches!(
            key_from_document(&doc, &credential_kid, AUTHORITY),
            Err(BundleError::WrongSigner { .. })
        ));
        // even with the credential key in hand, verify refuses it
        assert!(matches!(
            verify(&bytes, &jws, &credential_key.public_jwk(), expected()),
            Err(BundleError::WrongSigner { .. })
        ));
        // the bundle key of the same document is accepted
        let jws = sign_manifest(&bytes, &bundle_key, &kid()).unwrap();
        let jwk =
            key_from_document(&doc, &signing_kid(&jws, AUTHORITY).unwrap(), AUTHORITY).unwrap();
        assert!(verify(&bytes, &jws, jwk, expected()).is_ok());
    }

    /// TEST ONLY RSA key (PKCS#1 DER) of the PS256 test.
    const TEST_RSA_KEY: &[u8] = include_bytes!("../tests/fixtures/test-only-rsa-2048.der");

    #[test]
    fn a_ps256_bundle_signature_is_refused() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
        use ring::signature::{RSA_PSS_SHA256, RsaKeyPair, RsaPublicKeyComponents};
        let pair = RsaKeyPair::from_der(TEST_RSA_KEY).unwrap();
        let public = RsaPublicKeyComponents::<Vec<u8>>::from(pair.public());
        let rsa_jwk = Jwk::RSA {
            n: B64.encode(&public.n),
            e: B64.encode(&public.e),
        };
        // a valid PS256 signature by the key listed as #bundle-key
        let bytes = serde_json::to_vec(&manifest()).unwrap();
        let header = serde_json::json!({"alg": "PS256", "typ": JWS_TYPE, "kid": kid()});
        let input = format!(
            "{}.{}",
            B64.encode(serde_json::to_vec(&header).unwrap()),
            B64.encode(&bytes)
        );
        let mut signature = vec![0; pair.public().modulus_len()];
        pair.sign(
            &RSA_PSS_SHA256,
            &ring::rand::SystemRandom::new(),
            input.as_bytes(),
            &mut signature,
        )
        .unwrap();
        let jws = format!("{input}.{}", B64.encode(&signature));
        // the signature itself is good ...
        Jws::parse(&jws).unwrap().verify(&rsa_jwk).unwrap();
        // ... but a bundle must be ES256
        assert_eq!(
            signing_kid(&jws, AUTHORITY),
            Err(BundleError::WrongAlgorithm(Alg::PS256))
        );
        assert_eq!(
            verify(&bytes, &jws, &rsa_jwk, expected()),
            Err(BundleError::WrongAlgorithm(Alg::PS256))
        );
    }

    #[test]
    fn only_the_pinned_version_is_accepted() {
        let key = SigningKey::generate().unwrap();
        let (bytes, jws) = signed(&key);
        let pinned = Expected {
            authority: AUTHORITY,
            version: "2026.12.0",
        };
        assert_eq!(
            verify(&bytes, &jws, &key.public_jwk(), pinned),
            Err(BundleError::WrongVersion {
                expected: "2026.12.0".to_owned(),
                found: "2026.11.0".to_owned()
            })
        );
    }

    #[test]
    fn other_tokens_of_the_key_are_not_bundle_signatures() {
        let key = SigningKey::generate().unwrap();
        let bytes = serde_json::to_vec(&manifest()).unwrap();
        let raw = RawValue::from_string(String::from_utf8(bytes.clone()).unwrap()).unwrap();
        let jws = key.sign_json(&kid(), "JWT", &raw).unwrap();
        assert_eq!(
            verify(&bytes, &jws, &key.public_jwk(), expected()),
            Err(BundleError::WrongType)
        );
    }

    #[test]
    fn files_are_checked() {
        let mut changed = files();
        changed.insert("shapes/public.ttl".to_owned(), b"changed".to_vec());
        assert_eq!(
            check_files(manifest(), changed),
            Err(BundleError::DigestMismatch("shapes/public.ttl".to_owned()))
        );
        let mut missing = files();
        missing.remove("contexts/dct.jsonld");
        assert_eq!(
            check_files(manifest(), missing),
            Err(BundleError::MissingFile("contexts/dct.jsonld".to_owned()))
        );
        let mut extra = files();
        extra.insert("shapes/extra.ttl".to_owned(), SHAPES.to_vec());
        assert_eq!(
            check_files(manifest(), extra),
            Err(BundleError::UnlistedFile("shapes/extra.ttl".to_owned()))
        );
    }

    #[test]
    fn paths_stay_inside_the_folder() {
        for bad in [
            "",
            "/etc/passwd",
            "../up.ttl",
            "shapes/../../up.ttl",
            "shapes/./a.ttl",
            "shapes//a.ttl",
            "shapes\\a.ttl",
            "C:/a.ttl",
            ".hidden.ttl",
            "manifest.json",
            "manifest.jws",
            "spaces in.ttl",
            "ünïcode.ttl",
        ] {
            assert!(check_path(bad).is_err(), "{bad:?} should be refused");
        }
        for good in ["a.ttl", "shapes/public-v1.ttl", "code_lists/x.nt"] {
            assert!(check_path(good).is_ok(), "{good:?} should be accepted");
        }
        let mut traversal = manifest();
        traversal.files[0].path = "../outside.ttl".to_owned();
        let bytes = serde_json::to_vec(&traversal).unwrap();
        assert!(matches!(
            Manifest::parse(&bytes),
            Err(BundleError::Path { .. })
        ));
    }

    /// The manifest of [`manifest`] as JSON, with one more file,
    /// `policies/odrl.jsonld`, whose role is `role`.
    fn with_a_file_of_role(role: &serde_json::Value) -> Vec<u8> {
        let mut json = serde_json::to_value(manifest()).unwrap();
        json["files"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "path": "policies/odrl.jsonld",
                "sha256": sha256_hex(b"not read"),
                "mediaType": "application/ld+json",
                "role": role
            }));
        serde_json::to_vec(&json).unwrap()
    }

    /// The media type of a file with an unknown role may be outside the
    /// three (it is kept as [`MediaType::Other`]); a file with a known
    /// role keeps the closed list.
    #[test]
    fn an_unknown_role_may_have_another_media_type_and_a_known_role_may_not() {
        let unknown_role = serde_json::json!({"type": "policies"});
        let mut json: serde_json::Value =
            serde_json::from_slice(&with_a_file_of_role(&unknown_role)).unwrap();
        json["files"][2]["mediaType"] = "application/odrl+json".into();
        let parsed = Manifest::parse(&serde_json::to_vec(&json).unwrap()).unwrap();
        assert_eq!(parsed.files[2].media_type, MediaType::Other);
        // one of the three is kept as it is
        json["files"][2]["mediaType"] = "text/turtle".into();
        let parsed = Manifest::parse(&serde_json::to_vec(&json).unwrap()).unwrap();
        assert_eq!(parsed.files[2].media_type, MediaType::Turtle);
        // not the shape of a media type: refused, also for an unknown role
        json["files"][2]["mediaType"] = "application/odrl+json; v=2".into();
        let error = Manifest::parse(&serde_json::to_vec(&json).unwrap()).unwrap_err();
        assert!(error.to_string().contains("RFC 6838"), "{error}");
        // the same media type on a file with a known role: refused
        json["files"][2]["mediaType"] = "application/odrl+json".into();
        json["files"][2]["role"] = serde_json::json!({"type": "vocabulary"});
        let error = Manifest::parse(&serde_json::to_vec(&json).unwrap()).unwrap_err();
        assert!(
            error.to_string().contains("policies/odrl.jsonld"),
            "{error}"
        );
        // a manifest is never written with it
        assert!(serde_json::to_string(&MediaType::Other).is_err());
        assert!(serde_json::from_str::<MediaType>("\"Other\"").is_err());
    }

    #[test]
    fn restricted_names_follow_rfc_6838() {
        for good in [
            "text/turtle",
            "a/b",
            "application/vnd.x-y_z.1+json",
            "1a/2b!#$&^",
        ] {
            assert!(is_media_type_shape(good), "{good}");
        }
        for bad in [
            "", "text", "text/", "/x", "a/b/c", "a /b", "a/b;c=d", "+a/b", "a/.b", "\u{e9}/b",
        ] {
            assert!(!is_media_type_shape(bad), "{bad}");
        }
    }

    /// ADR 0005, amended in 0.13: a file whose role type is not known
    /// is skipped, with whatever fields the role has; the other files
    /// are used.
    #[test]
    fn a_file_of_an_unknown_role_is_skipped_and_the_others_are_used() {
        let role = serde_json::json!({
            "type": "policies", "appliesTo": ["offers"], "level": {"strict": true}
        });
        let parsed = Manifest::parse(&with_a_file_of_role(&role)).unwrap();
        assert_eq!(parsed.files.len(), 3);
        assert_eq!(
            parsed.files[2].role,
            Role::Unknown {
                type_name: "policies".to_owned()
            }
        );
        let skipped = SkippedFile {
            path: "policies/odrl.jsonld",
            role_type: "policies",
        };
        assert_eq!(parsed.files_of_unknown_role(), [skipped]);
        let fetched: Vec<&str> = parsed
            .files_to_fetch()
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();
        assert_eq!(fetched, ["shapes/public.ttl", "contexts/dct.jsonld"]);

        // the skipped file's bytes are not needed ...
        let bundle = check_files(parsed.clone(), files()).unwrap();
        assert_eq!(bundle.files().len(), 2);
        assert_eq!(bundle.file("policies/odrl.jsonld"), None);
        assert_eq!(bundle.skipped_files(), [skipped]);
        assert_eq!(
            bundle.access_rights().into_iter().collect::<Vec<_>>(),
            [PUBLIC]
        );
        // ... and if they are given, they are dropped, whatever they are
        let mut with_bytes = files();
        with_bytes.insert("policies/odrl.jsonld".to_owned(), b"anything".to_vec());
        assert_eq!(check_files(parsed, with_bytes).unwrap(), bundle);
        // a known file is still needed
        let mut one_missing = files();
        one_missing.remove("contexts/dct.jsonld");
        let parsed = Manifest::parse(&with_a_file_of_role(
            &serde_json::json!({"type": "policies"}),
        ))
        .unwrap();
        assert_eq!(
            check_files(parsed, one_missing),
            Err(BundleError::MissingFile("contexts/dct.jsonld".to_owned()))
        );
    }

    /// With nothing understood the bundle would give no rules, so it is
    /// refused, also through [`verify`].
    #[test]
    fn a_manifest_with_only_unknown_roles_is_refused() {
        let mut json = serde_json::to_value(manifest()).unwrap();
        json["files"][0]["role"] = serde_json::json!({"type": "policies"});
        json["files"][1]["role"] = serde_json::json!({"type": "rules", "x": 1});
        let bytes = serde_json::to_vec(&json).unwrap();
        assert_eq!(Manifest::parse(&bytes), Err(BundleError::NoKnownRole));

        // signed by a later version of the build tool, which knows them
        let key = SigningKey::generate().unwrap();
        let jws = signed_by_a_later_version(&bytes, &key);
        assert_eq!(
            verify(&bytes, &jws, &key.public_jwk(), expected()),
            Err(BundleError::NoKnownRole)
        );
    }

    /// A good bundle signature over `bytes`, without the checks of this
    /// version's [`sign_manifest`].
    fn signed_by_a_later_version(bytes: &[u8], key: &SigningKey) -> String {
        let raw = RawValue::from_string(String::from_utf8(bytes.to_vec()).unwrap()).unwrap();
        key.sign_json(&kid(), JWS_TYPE, &raw).unwrap()
    }

    /// A reader skips an unknown role; the signer never publishes one.
    #[test]
    fn a_manifest_with_an_unknown_role_verifies_but_is_not_signed_here() {
        let bytes = with_a_file_of_role(&serde_json::json!({"type": "policies"}));
        let key = SigningKey::generate().unwrap();
        let refused = sign_manifest(&bytes, &key, &kid()).unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("policies/odrl.jsonld: the role policies"),
            "{refused}"
        );
        let jws = signed_by_a_later_version(&bytes, &key);
        let verified = verify(&bytes, &jws, &key.public_jwk(), expected()).unwrap();
        assert_eq!(verified.files_of_unknown_role().len(), 1);
    }

    /// Only a well-formed role object with an unknown type is skipped.
    /// Everything else about roles stays an error.
    #[test]
    fn only_a_well_formed_unknown_role_is_skipped() {
        use serde_json::json;
        let refused = [
            // no type, or a type that is not a string
            json!({}),
            json!({"url": "https://contexts.example/x"}),
            json!({"type": 7}),
            json!({"type": null}),
            json!({"type": ["policies"]}),
            // not an object
            json!("policies"),
            json!(null),
            json!(["policies"]),
            // a type that is not a name
            json!({"type": ""}),
            json!({"type": "two words"}),
            json!({"type": "9lives"}),
            json!({"type": "line\nbreak"}),
            json!({"type": "x".repeat(MAX_ROLE_TYPE_LEN + 1)}),
            // the names are exact: another case is another, unknown type,
            // but a known role with a field it does not have is refused
            json!({"type": "messages", "extra": 1}),
            json!({"type": "vocabulary", "accessRights": ["https://x.example"]}),
            json!({"type": "context"}),
            json!({"type": "context", "url": "https://contexts.example/x", "extra": 1}),
            json!({"type": "datasets"}),
        ];
        for role in refused {
            let outcome = Manifest::parse(&with_a_file_of_role(&role));
            assert!(
                matches!(outcome, Err(BundleError::Manifest(_))),
                "{role}: {outcome:?}"
            );
        }
        // a known type in another case is a mistake, not a later role
        for misspelt in ["Messages", "DATASETS", "Vocabulary", "conText"] {
            let outcome = Manifest::parse(&with_a_file_of_role(&json!({"type": misspelt})));
            let Err(BundleError::Manifest(reason)) = outcome else {
                panic!("{misspelt}: {outcome:?}");
            };
            assert!(reason.contains("the type is written"), "{reason}");
        }
        // a repeated `type` is refused, also when the last one is unknown
        let text = String::from_utf8(with_a_file_of_role(&json!({"type": "policies"}))).unwrap();
        let repeated = text.replace(
            r#"{"type":"policies"}"#,
            r#"{"type":"messages","type":"policies"}"#,
        );
        assert_ne!(text, repeated);
        assert!(Manifest::parse(repeated.as_bytes()).is_err());

        let skipped = [
            json!({"type": "policies"}),
            json!({"type": "policy-shapes_2", "anything": [1, {"a": null}]}),
            json!({"type": "x".repeat(MAX_ROLE_TYPE_LEN)}),
        ];
        for role in skipped {
            let parsed = Manifest::parse(&with_a_file_of_role(&role)).unwrap();
            assert!(!parsed.files[2].role.is_known(), "{role}");
        }
    }

    /// The other fields of a file with an unknown role are checked as
    /// for every file; of its media type only the shape is.
    #[test]
    fn a_file_of_an_unknown_role_is_still_a_strict_file_entry() {
        let good = with_a_file_of_role(&serde_json::json!({"type": "policies"}));
        let good: serde_json::Value = serde_json::from_slice(&good).unwrap();
        let mutations: Vec<fn(&mut serde_json::Value)> = vec![
            |m| m["files"][2]["extra"] = "x".into(),
            |m| m["files"][2]["path"] = "../x".into(),
            |m| m["files"][2]["path"] = "shapes/public.ttl".into(),
            |m| m["files"][2]["sha256"] = "ABC".into(),
            // not the shape of a media type (it has a parameter)
            |m| m["files"][2]["mediaType"] = "application/rdf+xml; charset=utf-8".into(),
        ];
        for (index, mutate) in mutations.into_iter().enumerate() {
            let mut changed = good.clone();
            mutate(&mut changed);
            let bytes = serde_json::to_vec(&changed).unwrap();
            assert!(Manifest::parse(&bytes).is_err(), "mutation {index}");
        }
        // any media type of the right shape, also outside the three
        for media_type in [
            "text/turtle",
            "application/n-triples",
            "application/rdf+xml",
        ] {
            let mut changed = good.clone();
            changed["files"][2]["mediaType"] = media_type.into();
            assert!(Manifest::parse(&serde_json::to_vec(&changed).unwrap()).is_ok());
        }
    }

    /// A manifest is written only with known roles: the other fields of
    /// an unknown role were not kept.
    #[test]
    fn an_unknown_role_is_never_written() {
        let parsed = Manifest::parse(&with_a_file_of_role(
            &serde_json::json!({"type": "policies"}),
        ))
        .unwrap();
        let error = serde_json::to_vec(&parsed).unwrap_err().to_string();
        assert!(error.contains("unknown role policies"), "{error}");
        // the known roles are written as they were read
        let written = serde_json::to_value(manifest()).unwrap();
        assert_eq!(
            written["files"][0]["role"],
            serde_json::json!({"type": "datasets", "accessRights": [PUBLIC]})
        );
        assert_eq!(
            Manifest::parse(&serde_json::to_vec(&written).unwrap()).unwrap(),
            manifest()
        );
    }

    #[test]
    fn manifests_are_read_strictly() {
        let good = serde_json::to_value(manifest()).unwrap();
        assert!(Manifest::parse(&serde_json::to_vec(&good).unwrap()).is_ok());
        let mutations: Vec<fn(&mut serde_json::Value)> = vec![
            |m| m["extra"] = "x".into(),
            |m| m["files"][0]["extra"] = "x".into(),
            |m| m["files"][0]["role"]["extra"] = "x".into(),
            |m| m["files"][0]["mediaType"] = "application/rdf+xml".into(),
            |m| m["files"][0]["sha256"] = "ABC".into(),
            |m| m["files"][0]["role"] = serde_json::json!({"type": "datasets", "accessRights": []}),
            |m| m["files"][0]["mediaType"] = "application/ld+json".into(),
            |m| m["files"][1]["role"]["url"] = "file:///etc/passwd".into(),
            |m| m["files"] = serde_json::json!([]),
            |m| m["version"] = "../1".into(),
            |m| m["issued"] = "14.11.2026".into(),
            |m| m["issued"] = "2026-11-14T10:00:00+01:00".into(),
            |m| m["issued"] = "2026-11-14T10:00Z".into(),
            |m| m["issued"] = "2026-11-14T10:00:00.5Z".into(),
            |m| m["issued"] = "2026-11-14Z".into(),
            |m| m["issued"] = "2026-11-14".into(),
            |m| m["authority"] = "did:key:z6Mk".into(),
            |m| m["profile"] = "not an iri".into(),
            |m| {
                let first = m["files"][0].clone();
                m["files"].as_array_mut().unwrap().push(first);
            },
        ];
        for (index, mutate) in mutations.into_iter().enumerate() {
            let mut changed = good.clone();
            mutate(&mut changed);
            let bytes = serde_json::to_vec(&changed).unwrap();
            assert!(Manifest::parse(&bytes).is_err(), "mutation {index}");
        }
        // a repeated key
        let repeated = br#"{"version":"1","version":"2","issued":"2026-11-14T00:00:00Z","authority":"did:web:a.example","profile":"https://p.example","files":[]}"#;
        assert!(Manifest::parse(repeated).is_err());
    }

    #[test]
    fn signing_refuses_white_space_around_the_json() {
        let key = SigningKey::generate().unwrap();
        let mut bytes = serde_json::to_vec(&manifest()).unwrap();
        bytes.push(b'\n');
        assert!(sign_manifest(&bytes, &key, &kid()).is_err());
    }

    #[test]
    fn issued_is_a_utc_date_and_time_in_order() {
        for good in ["2026-11-14T00:00:00Z", "2026-11-14T23:59:59Z"] {
            assert!(issued_is_valid(good), "{good}");
        }
        for bad in [
            "",
            "2026-11-14",
            "2026-11-14T10:00:00",
            "2026-11-14T10:00Z",
            "2026-11-14T10:00:00+01:00",
            "2026-11-14T10:00:00.5Z",
            "2026-11-14T25:00:00Z",
            "2026-02-30T00:00:00Z",
        ] {
            assert!(!issued_is_valid(bad), "{bad}");
        }
        // the text order is the time order
        let order = [
            "2026-11-13T23:59:59Z",
            "2026-11-14T00:00:00Z",
            "2026-11-14T09:30:00Z",
            "2026-11-15T00:00:00Z",
        ];
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
