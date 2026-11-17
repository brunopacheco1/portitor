// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The two pieces of HTTP text that every Portitor service reads from
//! other parties (`portitor_runtime::http`): the `Authorization` header
//! (`bearer_token`), and the URLs other participants choose, such as a
//! `callbackAddress` (`outbound_url_allowed`). Nothing may panic; a token
//! is never empty and has no spaces around it; and a URL is allowed only
//! with an `https://` scheme (or `http://` when the operator allows it),
//! a host, and no user part.
#![no_main]

use libfuzzer_sys::fuzz_target;
use portitor_runtime::http::{bearer_token, outbound_url_allowed};

fn check_bearer_token(header_value: &str) {
    let Some(token) = bearer_token(header_value) else {
        return;
    };
    assert!(!token.is_empty());
    assert_eq!(token, token.trim());
    let (scheme, _) = header_value.split_once(' ').expect("a scheme and a token");
    assert!(scheme.eq_ignore_ascii_case("bearer"));
}

/// The host part of an allowed URL: after the scheme, up to the first
/// `/`, `?` or `#`.
fn host_of(url: &str) -> &str {
    // the scheme is ASCII, so its length is the same in any case
    let scheme_length = if url.to_ascii_lowercase().starts_with("https://") {
        "https://".len()
    } else {
        "http://".len()
    };
    let after_scheme = &url[scheme_length..];
    after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
}

fn check_url(url: &str) {
    let https_only = outbound_url_allowed(url, false);
    let with_http = outbound_url_allowed(url, true);
    // allowing http:// never refuses a URL that was allowed without it
    assert!(!https_only || with_http);
    if https_only {
        assert!(url.to_ascii_lowercase().starts_with("https://"));
    }
    if with_http {
        let lower = url.to_ascii_lowercase();
        assert!(lower.starts_with("https://") || lower.starts_with("http://"));
        let host = host_of(url);
        assert!(!host.is_empty());
        assert!(!host.contains('@'));
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    check_bearer_token(text);
    check_url(text);
});
