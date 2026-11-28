// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Adapter: resolves `did:web` DIDs over HTTPS (W3C did:web method,
//! "Read (Resolve)"), with a short cache (ADR 0016, amended in 0.12:
//! `PORTITOR_DID_CACHE_SECONDS`, 5 minutes by default, `0` for no cache).
//!
//! The cache is a trade: while a document is kept, a key its owner has
//! removed from it is still accepted (R-41). The setting lets an operator
//! choose how long, in every component that resolves DIDs (connector,
//! identity, issuer).
//!
//! The DID comes from the other party (the `iss` of its token), so the URL
//! fetched is the caller's choice. The same rules apply as to every
//! outgoing call ([`crate::http`]): HTTPS only, a time limit, a size
//! limit, no redirects (hyper does not follow them). Plain HTTP is only
//! used with `allow_http`, for local tests and the DCP TCK, whose DIDs
//! (`did:web:localhost%3A...`) are served over plain HTTP.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portitor_dcp::{DidDocument, did_web_url};

use crate::http::HttpSender;
use crate::internal_addresses::InternalAddresses;
use crate::settings::bounded_number;

/// The setting for how long a resolved DID document is reused.
pub const CACHE_SETTING: &str = "PORTITOR_DID_CACHE_SECONDS";

/// How long a resolved DID document is reused by default: 5 minutes.
pub const DEFAULT_SECONDS: u64 = 300;

/// The longest a DID document may be reused: one hour. A larger setting
/// is refused at start.
pub const MAX_SECONDS: u64 = 3_600;

/// [`DEFAULT_SECONDS`] as a [`Duration`].
pub const DEFAULT_TTL: Duration = Duration::from_secs(DEFAULT_SECONDS);

/// Reads `PORTITOR_DID_CACHE_SECONDS` (`value`, `None` when it is not
/// set): whole seconds from 0 to [`MAX_SECONDS`]; [`DEFAULT_SECONDS`]
/// when it is not set or empty ([`bounded_number`]). `0` turns the cache
/// off: every use fetches the document.
///
/// # Errors
/// Not a whole number in that range; the text names the setting.
pub fn cache_seconds(value: Option<&str>) -> Result<u64, String> {
    bounded_number(CACHE_SETTING, value, 0..=MAX_SECONDS, DEFAULT_SECONDS)
}

/// [`cache_seconds`] of the environment variable
/// `PORTITOR_DID_CACHE_SECONDS`: what each component reads at start.
///
/// # Errors
/// See [`cache_seconds`].
pub fn cache_seconds_from_env() -> Result<u64, String> {
    cache_seconds(crate::settings::text_from_env(CACHE_SETTING).as_deref())
}

/// The most DID documents kept at once. Callers choose the DIDs, so
/// without a limit the cache could grow with every new caller.
pub const MAX_CACHED: usize = 1_000;

/// Why a DID could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveError {
    /// What went wrong, for the log.
    pub reason: String,
    /// Whether trying again later may help: the DID's host did not
    /// answer, or answered 408, 429 or 5xx
    /// ([`crate::http::HttpError::is_temporary`]). A DID that is not
    /// `did:web`, a 404 and a document that is not valid are final.
    pub temporary: bool,
}

impl ResolveError {
    /// A final failure: trying again will not help.
    fn rejected(reason: String) -> Self {
        Self {
            reason,
            temporary: false,
        }
    }

    /// Whether trying again later may help (see the `temporary` field).
    #[must_use]
    pub fn is_temporary(&self) -> bool {
        self.temporary
    }
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for ResolveError {}

/// Lets `?` turn a [`ResolveError`] into its text, in code whose errors
/// are text.
impl From<ResolveError> for String {
    fn from(error: ResolveError) -> Self {
        error.reason
    }
}

/// One document in the cache, and when it was fetched.
#[derive(Debug)]
struct Cached {
    fetched_at: Instant,
    document: Arc<DidDocument>,
}

/// Resolves `did:web` DIDs and keeps each document for a while.
#[derive(Debug)]
pub struct DidWebResolver {
    http: HttpSender,
    allow_http: bool,
    ttl: Duration,
    max_cached: usize,
    /// The documents fetched, by DID. A poisoned lock (a thread panicked
    /// while holding it) is not used again: resolving then goes without
    /// the cache, which is slower but still correct.
    cache: Mutex<HashMap<String, Cached>>,
}

impl DidWebResolver {
    /// A resolver that reuses documents for `ttl`
    /// (`PORTITOR_DID_CACHE_SECONDS`); a `ttl` of zero keeps nothing.
    /// With `allow_http`, documents are fetched over plain HTTP (tests
    /// only). For our own DID, which the operator chose: the block on
    /// internal addresses does not apply ([`DidWebResolver::for_partners`]).
    ///
    /// # Errors
    /// The HTTP client cannot be built (see [`HttpSender::new`]).
    pub fn new(allow_http: bool, ttl: Duration) -> Result<Self, String> {
        Self::for_partners(allow_http, ttl, &InternalAddresses::default())
    }

    /// Like [`DidWebResolver::new`], for the DIDs of other parties
    /// (callers, holders, credential issuers): a document is fetched only
    /// from an address that `internal_addresses` allows (R-17;
    /// [`HttpSender::for_partners`]). [`DidWebResolver::new`] is for our
    /// own DID only.
    ///
    /// # Errors
    /// The HTTP client cannot be built (see [`HttpSender::new`]).
    pub fn for_partners(
        allow_http: bool,
        ttl: Duration,
        internal_addresses: &InternalAddresses,
    ) -> Result<Self, String> {
        Ok(Self {
            http: HttpSender::for_partners(allow_http, internal_addresses)?,
            allow_http,
            ttl,
            max_cached: MAX_CACHED,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// This resolver without spans or measures for its fetches
    /// ([`HttpSender::unobserved`]): for a readiness check.
    #[must_use]
    pub fn unobserved(self) -> Self {
        Self {
            http: self.http.unobserved(),
            ..self
        }
    }

    /// The DID document of `did`, from the cache or fetched.
    ///
    /// # Errors
    /// Not a valid `did:web` DID, the document cannot be fetched or read,
    /// or its `id` is another DID. [`ResolveError::is_temporary`] tells
    /// whether trying again later may help.
    pub async fn resolve(&self, did: &str) -> Result<Arc<DidDocument>, ResolveError> {
        if let Some(doc) = self.cached(did) {
            return Ok(doc);
        }
        let url = self.url(did).map_err(ResolveError::rejected)?;
        let json = match self.http.get_json(&url).await {
            Ok(json) => json,
            Err(error) => {
                return Err(ResolveError {
                    reason: error.to_string(),
                    temporary: error.is_temporary(),
                });
            }
        };
        let doc: DidDocument = serde_json::from_value(json)
            .map_err(|e| ResolveError::rejected(format!("DID document of {did}: {e}")))?;
        if doc.id != did {
            let reason = format!("the DID document at {url} is for {}", doc.id);
            return Err(ResolveError::rejected(reason));
        }
        let doc = Arc::new(doc);
        self.keep(did, &doc);
        Ok(doc)
    }

    /// Puts `doc` in the cache. When the cache is full, the expired
    /// documents go first, then the oldest one. Both look at every entry,
    /// but only when the cache is full.
    fn keep(&self, did: &str, doc: &Arc<DidDocument>) {
        if self.ttl.is_zero() {
            // the cache is off: a kept document would never be used
            return;
        }
        let Ok(mut cache) = self.cache.lock() else {
            // a poisoned lock: no cache (see the `cache` field)
            return;
        };
        if cache.len() >= self.max_cached && !cache.contains_key(did) {
            cache.retain(|_, entry| entry.fetched_at.elapsed() < self.ttl);
        }
        if cache.len() >= self.max_cached && !cache.contains_key(did) {
            let oldest = cache
                .iter()
                .min_by_key(|(_, entry)| entry.fetched_at)
                .map(|(oldest, _)| oldest.clone());
            if let Some(oldest) = oldest {
                cache.remove(&oldest);
            }
        }
        let entry = Cached {
            fetched_at: Instant::now(),
            document: Arc::clone(doc),
        };
        cache.insert(did.to_owned(), entry);
    }

    /// The cached document of `did`, if it is there and still fresh. A
    /// stale one is removed.
    fn cached(&self, did: &str) -> Option<Arc<DidDocument>> {
        // a poisoned lock: no cache (see the `cache` field)
        let mut cache = self.cache.lock().ok()?;
        match cache.get(did) {
            Some(entry) if entry.fetched_at.elapsed() < self.ttl => {
                Some(Arc::clone(&entry.document))
            }
            Some(_) => {
                cache.remove(did);
                None
            }
            None => None,
        }
    }

    fn url(&self, did: &str) -> Result<String, String> {
        let url = did_web_url(did).map_err(|e| e.to_string())?;
        if self.allow_http {
            Ok(url.replacen("https://", "http://", 1))
        } else {
            Ok(url)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Serves `body` at `/.well-known/did.json` and counts the requests.
    /// Unless `body` has an `id`, it gets the served DID as its `id`.
    async fn serve(mut body: serde_json::Value) -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let did = format!("did:web:127.0.0.1%3A{port}");
        if body.get("id").is_none() {
            body["id"] = serde_json::Value::from(did.clone());
        }
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let app = axum::Router::new().route(
            "/.well-known/did.json",
            axum::routing::get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                let body = body.clone();
                async move { axum::Json(body) }
            }),
        );
        tokio::spawn(async move { axum::serve(listener, app).await });
        (did, hits)
    }

    #[tokio::test]
    async fn resolves_and_caches() {
        let (did, hits) = serve(serde_json::json!({"verificationMethod": []})).await;
        let resolver = DidWebResolver::new(true, DEFAULT_TTL).unwrap();
        let first = resolver.resolve(&did).await.unwrap();
        let second = resolver.resolve(&did).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "the second resolve uses the cache"
        );
    }

    /// R-17: a resolver for other parties' DIDs fetches nothing from an
    /// internal address while the block is on, unless it is allowed; the
    /// refusal is not temporary and names the setting.
    #[tokio::test]
    async fn a_partner_did_on_an_internal_address_is_refused_with_the_block_on() {
        use crate::internal_addresses::{ALLOWED_SETTING, BLOCK_SETTING};
        let (did, hits) = serve(serde_json::json!({"verificationMethod": []})).await;
        let rule = |settings: &[(&str, &str)]| {
            InternalAddresses::from_vars(crate::testing::vars(settings)).unwrap()
        };
        let blocking = rule(&[(BLOCK_SETTING, "true")]);
        let resolver = DidWebResolver::for_partners(true, DEFAULT_TTL, &blocking).unwrap();
        let error = resolver.resolve(&did).await.unwrap_err();
        assert!(error.reason.contains(ALLOWED_SETTING), "{}", error.reason);
        assert!(!error.is_temporary());
        assert_eq!(hits.load(Ordering::SeqCst), 0, "nothing was fetched");

        let allowed = rule(&[(BLOCK_SETTING, "true"), (ALLOWED_SETTING, "127.0.0.1")]);
        let resolver = DidWebResolver::for_partners(true, DEFAULT_TTL, &allowed).unwrap();
        resolver.resolve(&did).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    /// `PORTITOR_DID_CACHE_SECONDS=0`: every resolve fetches the document,
    /// and nothing is kept.
    #[tokio::test]
    async fn a_cache_time_of_zero_fetches_every_time() {
        let (did, hits) = serve(serde_json::json!({"verificationMethod": []})).await;
        let seconds = cache_seconds(Some("0")).unwrap();
        let resolver = DidWebResolver::new(true, Duration::from_secs(seconds)).unwrap();
        resolver.resolve(&did).await.unwrap();
        resolver.resolve(&did).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 2, "no cache");
        assert!(resolver.cache.lock().unwrap().is_empty());
    }

    /// The default, 300 seconds: a document is reused until it is 300
    /// seconds old, then fetched again.
    #[tokio::test]
    async fn the_default_cache_time_is_300_seconds() {
        let (did, hits) = serve(serde_json::json!({"verificationMethod": []})).await;
        let seconds = cache_seconds(None).unwrap();
        assert_eq!(seconds, 300);
        let resolver = DidWebResolver::new(true, Duration::from_secs(seconds)).unwrap();
        resolver.resolve(&did).await.unwrap();
        let make_older = |age_seconds: u64| {
            let mut cache = resolver.cache.lock().unwrap();
            let entry = cache.get_mut(&did).unwrap();
            entry.fetched_at = Instant::now()
                .checked_sub(Duration::from_secs(age_seconds))
                .unwrap();
        };
        make_older(290);
        resolver.resolve(&did).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 1, "still fresh at 290 s");
        make_older(301);
        resolver.resolve(&did).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 2, "fetched again after 300 s");
    }

    #[test]
    fn the_cache_setting_is_bounded() {
        assert_eq!(cache_seconds(None), Ok(300));
        // empty is not set, as for every number setting
        assert_eq!(cache_seconds(Some("")), Ok(300));
        assert_eq!(cache_seconds(Some("0")), Ok(0));
        assert_eq!(cache_seconds(Some(" 3600 ")), Ok(3600));
        for refused in ["3601", "-1", "5m"] {
            let err = cache_seconds(Some(refused)).unwrap_err();
            assert!(
                err.starts_with("PORTITOR_DID_CACHE_SECONDS has an invalid value"),
                "{refused}: {err}"
            );
        }
    }

    /// The identity and the issuer resolve DIDs through a
    /// [`crate::directory::WebDirectory`]: the cache time they read from
    /// `PORTITOR_DID_CACHE_SECONDS` reaches its resolver.
    #[tokio::test]
    async fn the_cache_time_reaches_the_directory() {
        use crate::directory::{Directory as _, WebDirectory};

        for (seconds, expected_fetches) in [(0, 3), (300, 1)] {
            let (did, hits) = serve(serde_json::json!({"verificationMethod": []})).await;
            let directory = WebDirectory::new(true, seconds).unwrap();
            for _ in 0..3 {
                directory.did_document(&did).await.unwrap();
            }
            assert_eq!(
                hits.load(Ordering::SeqCst),
                expected_fetches,
                "{seconds} seconds"
            );
        }
    }

    /// The longest cache time, one hour: a document is reused until it is
    /// 3600 seconds old, then fetched again.
    #[tokio::test]
    async fn at_the_longest_cache_time_a_document_is_reused_for_an_hour() {
        let (did, hits) = serve(serde_json::json!({"verificationMethod": []})).await;
        let seconds = cache_seconds(Some("3600")).unwrap();
        let resolver = DidWebResolver::new(true, Duration::from_secs(seconds)).unwrap();
        resolver.resolve(&did).await.unwrap();
        let make_older = |age_seconds: u64| {
            let mut cache = resolver.cache.lock().unwrap();
            let entry = cache.get_mut(&did).unwrap();
            entry.fetched_at = Instant::now()
                .checked_sub(Duration::from_secs(age_seconds))
                .unwrap();
        };
        make_older(3_590);
        resolver.resolve(&did).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 1, "still fresh at 3590 s");
        make_older(3_601);
        resolver.resolve(&did).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 2, "fetched again after 3600 s");
    }

    /// Values at and around the bounds, and texts that are no whole
    /// number of seconds.
    #[test]
    fn the_cache_setting_takes_only_whole_seconds_from_0_to_3600() {
        for (accepted, seconds) in [("0", 0), ("1", 1), ("3599", 3599), ("3600", 3600)] {
            assert_eq!(cache_seconds(Some(accepted)), Ok(seconds), "{accepted}");
        }
        for refused in [
            "3601",
            "86400",
            "18446744073709551616",
            "-0",
            "300.0",
            "3e2",
            "0x10",
            "300s",
            "five",
            "300 600",
        ] {
            assert!(cache_seconds(Some(refused)).is_err(), "{refused}");
        }
    }

    /// A panic while the cache's lock was held (TD-41) leaves the
    /// resolver working: it resolves without a cache, every time.
    #[tokio::test]
    async fn a_poisoned_cache_lock_means_no_cache_but_still_an_answer() {
        let (did, hits) = serve(serde_json::json!({"verificationMethod": []})).await;
        let resolver = Arc::new(DidWebResolver::new(true, DEFAULT_TTL).unwrap());
        let held = Arc::clone(&resolver);
        let panicked = std::thread::spawn(move || {
            let _cache = held.cache.lock().unwrap();
            panic!("while the lock is held");
        })
        .join();
        assert!(panicked.is_err());
        assert!(resolver.cache.is_poisoned());

        resolver.resolve(&did).await.unwrap();
        resolver.resolve(&did).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn the_cache_is_bounded_and_drops_the_oldest_first() {
        let (first, first_hits) = serve(serde_json::json!({"verificationMethod": []})).await;
        let (second, _) = serve(serde_json::json!({"verificationMethod": []})).await;
        let (third, _) = serve(serde_json::json!({"verificationMethod": []})).await;
        let mut resolver = DidWebResolver::new(true, DEFAULT_TTL).unwrap();
        resolver.max_cached = 2;
        resolver.resolve(&first).await.unwrap();
        resolver.resolve(&second).await.unwrap();
        resolver.resolve(&third).await.unwrap();
        let cached: Vec<String> = resolver.cache.lock().unwrap().keys().cloned().collect();
        assert_eq!(cached.len(), 2);
        assert!(!cached.contains(&first), "the oldest went: {cached:?}");
        resolver.resolve(&first).await.unwrap();
        assert_eq!(first_hits.load(Ordering::SeqCst), 2, "fetched again");
    }

    #[tokio::test]
    async fn a_full_cache_drops_every_expired_document() {
        let mut dids = Vec::new();
        for _ in 0..4 {
            dids.push(serve(serde_json::json!({"verificationMethod": []})).await.0);
        }
        let mut resolver = DidWebResolver::new(true, Duration::from_secs(60)).unwrap();
        resolver.max_cached = 3;
        for did in &dids[..3] {
            resolver.resolve(did).await.unwrap();
        }
        // the first two have expired
        for did in &dids[..2] {
            resolver
                .cache
                .lock()
                .unwrap()
                .get_mut(did)
                .unwrap()
                .fetched_at = Instant::now()
                .checked_sub(Duration::from_secs(120))
                .unwrap();
        }
        resolver.resolve(&dids[3]).await.unwrap();
        let mut cached: Vec<String> = resolver.cache.lock().unwrap().keys().cloned().collect();
        cached.sort();
        let mut expected = vec![dids[2].clone(), dids[3].clone()];
        expected.sort();
        assert_eq!(cached, expected, "both expired documents went");
    }

    #[tokio::test]
    async fn refuses_a_document_for_another_did() {
        let (did, _) = serve(serde_json::json!({"id": "did:web:someone.else"})).await;
        let resolver = DidWebResolver::new(true, DEFAULT_TTL).unwrap();
        let err = resolver.resolve(&did).await.unwrap_err();
        assert!(err.reason.contains("is for did:web:someone.else"), "{err}");
        assert!(!err.is_temporary());
    }

    /// A host that does not answer, or answers 5xx, may answer later; a
    /// 404 and a DID that is not `did:web` will not change.
    #[tokio::test]
    async fn tells_temporary_failures_from_final_ones() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = axum::Router::new()
            .route(
                "/down/did.json",
                axum::routing::get(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE }),
            )
            .route(
                "/gone/did.json",
                axum::routing::get(|| async { axum::http::StatusCode::NOT_FOUND }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await });
        let resolver = DidWebResolver::new(true, DEFAULT_TTL).unwrap();
        let host = format!("did:web:127.0.0.1%3A{port}");
        let down = resolver.resolve(&format!("{host}:down")).await.unwrap_err();
        assert!(down.is_temporary(), "{down}");
        let gone = resolver.resolve(&format!("{host}:gone")).await.unwrap_err();
        assert!(!gone.is_temporary(), "{gone}");
        // nobody listens on port 9 (discard)
        let unreachable = resolver.resolve("did:web:127.0.0.1%3A9").await.unwrap_err();
        assert!(unreachable.is_temporary(), "{unreachable}");
        let not_web = resolver.resolve("did:key:z6Mk").await.unwrap_err();
        assert!(!not_web.is_temporary(), "{not_web}");
    }

    #[tokio::test]
    async fn https_only_by_default() {
        let resolver = DidWebResolver::new(false, DEFAULT_TTL).unwrap();
        assert_eq!(
            resolver.url("did:web:example.com").unwrap(),
            "https://example.com/.well-known/did.json"
        );
        assert!(resolver.resolve("did:key:z6Mk").await.is_err());
    }
}
