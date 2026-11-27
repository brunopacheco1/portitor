// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The status list cache (ADR 0016, amended in 0.11; TD-06): of the
//! connector's DCP verifier, and since 1.4 of the identity's credential
//! status check (TD-20). Moved here from
//! `portitor-connector/src/auth/dcp/status_list_cache.rs` in 1.4.
//!
//! A status list credential is kept for `PORTITOR_STATUS_LIST_CACHE_SECONDS`
//! on the connector (default [`DEFAULT_SECONDS`], at most
//! [`MAX_SECONDS`]), keyed by its URL
//! (`statusListCredential`). Only a list that was fetched and verified
//! (signed by its issuer, valid now) is kept; a failed fetch or check is
//! never kept. A list is never kept past its own end (`validUntil`, or
//! the JWT's `exp`, whichever comes first). 0 turns the cache off: each
//! check fetches the list.
//!
//! The cost: a revocation takes up to that long to be seen (DEPLOYING).
//!
//! The cache lives in memory. It holds at most [`MAX_LISTS`] lists. Two
//! requests that miss at the same time both fetch the list, and the
//! later one is kept; that costs one fetch more, never a wrong answer.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use portitor_dcp::credential::Credential;

/// How long a status list is kept by default: 5 minutes.
pub const DEFAULT_SECONDS: u64 = 300;

/// The longest a status list may be kept: one hour. A larger setting is
/// refused at start.
pub const MAX_SECONDS: u64 = 3_600;

/// The most status lists kept at once. Only trusted issuers name status
/// lists that are fetched (the verifier skips the others before), so a
/// data space has few; the limit only bounds memory.
pub const MAX_LISTS: usize = 256;

/// One verified status list credential, and when it may be used.
#[derive(Debug)]
struct Cached {
    list: Arc<Credential>,
    /// When it was fetched (seconds since 1970).
    fetched_at: u64,
    /// The first second it may no longer be used.
    fresh_until: u64,
}

impl Cached {
    /// Whether the list may still be used at `now`. A clock set back
    /// before the fetch makes it stale too.
    fn is_fresh(&self, now: u64) -> bool {
        self.fetched_at <= now && now < self.fresh_until
    }
}

/// Verified status list credentials, by URL.
#[derive(Debug)]
pub struct StatusListCache {
    /// How long a list is kept, in seconds; 0: never.
    seconds: u64,
    /// The lists. A poisoned lock (a thread panicked while holding it) is
    /// not used again: lists are then fetched each time, which is slower
    /// but still correct.
    lists: Mutex<HashMap<String, Cached>>,
}

impl StatusListCache {
    /// A cache that keeps each list for `seconds` (0: no cache).
    #[must_use]
    pub fn new(seconds: u64) -> Self {
        Self {
            seconds,
            lists: Mutex::new(HashMap::new()),
        }
    }

    /// The list kept for `url`, if it may still be used at `now`. A stale
    /// one is removed.
    pub fn get(&self, url: &str, now: u64) -> Option<Arc<Credential>> {
        if self.seconds == 0 {
            return None;
        }
        // a poisoned lock: no cache (see the `lists` field)
        let mut lists = self.lists.lock().ok()?;
        match lists.get(url) {
            Some(cached) if cached.is_fresh(now) => Some(Arc::clone(&cached.list)),
            Some(_) => {
                lists.remove(url);
                None
            }
            None => None,
        }
    }

    /// Keeps `list`, fetched from `url` and verified at `now`. Nothing is
    /// kept when the cache is off, when the list ends now, or when the
    /// cache is full of lists still fresh.
    pub fn keep(&self, url: &str, list: Arc<Credential>, now: u64) {
        let fresh_until = self.fresh_until(&list, now);
        if fresh_until <= now {
            return;
        }
        let Ok(mut lists) = self.lists.lock() else {
            // a poisoned lock: no cache (see the `lists` field)
            return;
        };
        if lists.len() >= MAX_LISTS && !lists.contains_key(url) {
            lists.retain(|_, cached| cached.is_fresh(now));
            if lists.len() >= MAX_LISTS {
                return;
            }
        }
        let cached = Cached {
            list,
            fetched_at: now,
            fresh_until,
        };
        lists.insert(url.to_owned(), cached);
    }

    /// The first second `list`, fetched at `now`, may no longer be used:
    /// after the configured time, or at the list's own end, whichever
    /// comes first.
    fn fresh_until(&self, list: &Credential, now: u64) -> u64 {
        let by_setting = now.saturating_add(self.seconds);
        // `validity.until` is the earlier of `validUntil` (or
        // `expirationDate`) and the JWT's `exp`
        match list.validity.until {
            Some(until) => {
                // an end before 1970 is in the past: 0
                let until = u64::try_from(until).unwrap_or(0);
                by_setting.min(until)
            }
            None => by_setting,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use portitor_dcp::SigningKey;
    use serde_json::json;

    use super::*;

    const URL: &str = "https://issuer.example/status/1";

    /// A status list credential JWT with the JWT claims `extra` (for
    /// example `exp`). Its signature is not checked here.
    fn list(extra: &serde_json::Value) -> Arc<Credential> {
        let key = SigningKey::generate().unwrap();
        let mut payload = json!({
            "iss": "did:web:issuer.example",
            "vc": {
                "@context": ["https://www.w3.org/2018/credentials/v1"],
                "type": ["VerifiableCredential", "BitstringStatusListCredential"],
                "issuer": "did:web:issuer.example",
                "issuanceDate": "2026-01-01T00:00:00Z",
                "credentialSubject": {"id": "urn:uuid:list"}
            }
        });
        for (name, value) in extra.as_object().into_iter().flatten() {
            payload[name] = value.clone();
        }
        let jwt = key
            .sign_json("did:web:issuer.example#key-1", "JWT", &payload)
            .unwrap();
        Arc::new(Credential::parse(&jwt).unwrap())
    }

    const NOW: u64 = 1_800_000_000;

    #[test]
    fn at_zero_nothing_is_kept() {
        let cache = StatusListCache::new(0);
        cache.keep(URL, list(&json!({})), NOW);
        assert!(cache.get(URL, NOW).is_none());
    }

    #[test]
    fn a_list_is_kept_for_the_configured_time() {
        let cache = StatusListCache::new(300);
        cache.keep(URL, list(&json!({})), NOW);
        assert!(cache.get(URL, NOW).is_some());
        assert!(cache.get(URL, NOW + 299).is_some());
        assert!(cache.get(URL, NOW + 300).is_none());
        // removed once stale
        assert!(cache.get(URL, NOW + 1).is_none());
        assert!(cache.get("https://issuer.example/status/2", NOW).is_none());
    }

    #[test]
    fn a_list_is_never_kept_past_its_own_end() {
        let cache = StatusListCache::new(300);
        cache.keep(URL, list(&json!({"exp": NOW + 100})), NOW);
        assert!(cache.get(URL, NOW + 99).is_some());
        assert!(cache.get(URL, NOW + 100).is_none());
        // a list that ends now is not kept at all
        cache.keep(URL, list(&json!({"exp": NOW})), NOW);
        assert!(cache.get(URL, NOW).is_none());
    }

    #[test]
    fn a_clock_set_back_makes_a_list_stale() {
        let cache = StatusListCache::new(300);
        cache.keep(URL, list(&json!({})), NOW);
        assert!(cache.get(URL, NOW - 1).is_none());
    }

    #[test]
    fn a_full_cache_drops_stale_lists_and_never_grows_past_its_limit() {
        let cache = StatusListCache::new(300);
        let one = list(&json!({}));
        for index in 0..MAX_LISTS {
            cache.keep(&format!("{URL}/{index}"), Arc::clone(&one), NOW);
        }
        // full of fresh lists: a new one is not kept
        cache.keep(URL, Arc::clone(&one), NOW);
        assert!(cache.get(URL, NOW).is_none());
        // later, the stale ones make room
        cache.keep(URL, one, NOW + 300);
        assert!(cache.get(URL, NOW + 300).is_some());
        assert_eq!(cache.lists.lock().unwrap().len(), 1);
    }
}
