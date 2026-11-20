// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Wrong calls to a management API, the same ones for the three
//! binaries: calls without the API key, bodies that are not what the
//! route reads, and paths and queries with odd values.
//!
//! A test of a binary lists its routes as [`Call`]s and asks for the
//! answers ([`without_the_key`], [`with_a_wrong_body`],
//! [`with_odd_paths_and_queries`]). Each answer says which call it was
//! and the HTTP status; the test then says which statuses it allows
//! ([`statuses_other_than`]).
//!
//! The calls run in memory, without a network. A handler that panics
//! fails the test that asked.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, header};
use serde_json::{Value, json};
use tower::ServiceExt as _;

/// The length of a "very long" text: longer than any name or address,
/// and well under the size limit of a request.
pub const VERY_LONG: usize = 100_000;

/// The length of a very long part of a path or query: an address of
/// more than about 65 000 letters cannot be written as a request.
pub const VERY_LONG_IN_AN_ADDRESS: usize = 8_000;

/// One route of a management API, with real values in its path.
#[derive(Debug, Clone)]
pub struct Call {
    /// `GET`, `POST` or `DELETE`.
    pub method: &'static str,
    /// The path, for example `/v1/negotiations/urn:uuid:1`.
    pub path: &'static str,
    /// A body the route accepts, when the route reads a body.
    pub body: Option<Value>,
    /// Members of the body that had another name in an earlier release:
    /// (the name now, the name before).
    pub old_names: &'static [(&'static str, &'static str)],
}

impl Call {
    /// A route that reads no body.
    #[must_use]
    pub fn without_body(method: &'static str, path: &'static str) -> Self {
        Self {
            method,
            path,
            body: None,
            old_names: &[],
        }
    }

    /// A `POST` route and a body it accepts.
    #[must_use]
    pub fn post(path: &'static str, body: Value) -> Self {
        Self {
            method: "POST",
            path,
            body: Some(body),
            old_names: &[],
        }
    }
}

/// What a router answered to one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The call in words: `POST /v1/things, counterPartyId is a number`.
    pub call: String,
    /// The HTTP status.
    pub status: u16,
}

/// The answers whose status is not one of `allowed`, one per line: empty
/// when every answer is allowed.
#[must_use]
pub fn statuses_other_than(allowed: &[u16], answers: &[Answer]) -> String {
    let mut lines = Vec::new();
    for answer in answers {
        if !allowed.contains(&answer.status) {
            lines.push(format!("{}: {}\n", answer.call, answer.status));
        }
    }
    lines.concat()
}

/// Sends one request to `router` and gives the status.
async fn status_of(
    router: &Router,
    method: &str,
    path: &str,
    authorization: Option<&str>,
    body: Option<Vec<u8>>,
) -> u16 {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(value) = authorization {
        request = request.header(header::AUTHORIZATION, value);
    }
    let body = match body {
        Some(bytes) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(bytes)
        }
        None => Body::empty(),
    };
    let Ok(request) = request.body(body) else {
        // the test wrote a path or header that is not HTTP: no status
        return 0;
    };
    match router.clone().oneshot(request).await {
        Ok(response) => response.status().as_u16(),
        Err(never) => match never {},
    }
}

/// Every call of `calls` with its good body, but without the right
/// `Authorization` header: no header, another key, the key without
/// `Bearer`, and the key with one more letter.
pub async fn without_the_key(router: &Router, calls: &[Call], key: &str) -> Vec<Answer> {
    let headers = [
        ("no Authorization header", None),
        ("another key", Some("Bearer another-key".to_owned())),
        ("the key without Bearer", Some(key.to_owned())),
        ("the key as Basic", Some(format!("Basic {key}"))),
        (
            "the key and one more letter",
            Some(format!("Bearer {key}x")),
        ),
        ("an empty key", Some("Bearer ".to_owned())),
    ];
    let mut answers = Vec::new();
    for call in calls {
        for (name, value) in &headers {
            let body = call.body.as_ref().map(|body| body.to_string().into_bytes());
            let status = status_of(router, call.method, call.path, value.as_deref(), body).await;
            answers.push(Answer {
                call: format!("{} {}, {name}", call.method, call.path),
                status,
            });
        }
    }
    answers
}

/// The wrong bodies made from the good body `good`: (what is wrong, the
/// body). `old_names` as in [`Call::old_names`].
#[must_use]
pub fn wrong_bodies(good: &Value, old_names: &[(&str, &str)]) -> Vec<(String, Vec<u8>)> {
    let mut bodies: Vec<(String, Vec<u8>)> = vec![
        ("an empty body".to_owned(), Vec::new()),
        ("a body that is not JSON".to_owned(), b"{\"a\":".to_vec()),
        (
            "a body that is not UTF-8".to_owned(),
            vec![b'"', 0xff, b'"'],
        ),
        ("a JSON list".to_owned(), b"[]".to_vec()),
        ("a JSON text".to_owned(), b"\"text\"".to_vec()),
        ("a JSON number".to_owned(), b"7".to_vec()),
        ("JSON null".to_owned(), b"null".to_vec()),
    ];
    let mut add = |what: String, body: &Value| bodies.push((what, body.to_string().into_bytes()));

    let mut extra = good.clone();
    extra["notAMember"] = json!("x");
    add("a member the route does not know".to_owned(), &extra);

    for (now, before) in old_names {
        let mut renamed = good.clone();
        if let Some(members) = renamed.as_object_mut()
            && let Some(value) = members.remove(*now)
        {
            members.insert((*before).to_owned(), value);
        }
        add(format!("{now} under its old name {before}"), &renamed);
    }

    let Some(members) = good.as_object() else {
        return bodies;
    };
    for (name, value) in members {
        let mut changed = |what: &str, new: Value| {
            let mut body = good.clone();
            body[name] = new;
            add(format!("{name} {what}"), &body);
        };
        match value {
            Value::String(_) => {
                changed("is a number", json!(7));
                changed("is a list", json!(["x"]));
                changed("is an object", json!({"x": 1}));
                changed("is true", json!(true));
                changed("is empty", json!(""));
                changed("is only spaces", json!("   "));
                changed("is very long", json!("a".repeat(VERY_LONG)));
            }
            Value::Array(_) => {
                changed("is a text", json!("x"));
                changed("is an object", json!({"x": 1}));
                changed("holds a number", json!([7]));
                changed("holds an empty text", json!([""]));
                changed("holds a very long text", json!(["a".repeat(VERY_LONG)]));
            }
            Value::Object(_) => {
                changed("is a text", json!("x"));
                changed("is a list", json!([]));
                changed("is a number", json!(7));
            }
            Value::Number(_) | Value::Bool(_) | Value::Null => {
                changed("is a text", json!("x"));
            }
        }
    }
    bodies
}

/// Every call of `calls` that reads a body, with the key and each of the
/// [`wrong_bodies`] of its good body; and once with the good body but
/// without `Content-Type: application/json`.
pub async fn with_a_wrong_body(router: &Router, calls: &[Call], key: &str) -> Vec<Answer> {
    let authorization = format!("Bearer {key}");
    let mut answers = Vec::new();
    for call in calls {
        let Some(good) = &call.body else {
            continue;
        };
        for (what, body) in wrong_bodies(good, call.old_names) {
            let status = status_of(
                router,
                call.method,
                call.path,
                Some(&authorization),
                Some(body),
            )
            .await;
            answers.push(Answer {
                call: format!("{} {}, {what}", call.method, call.path),
                status,
            });
        }
    }
    answers
}

/// Every call of `calls`, with the key, with odd values in the place of
/// the last part of its path and in the query: very long, empty-looking,
/// percent-encoded bytes that are no text. Calls that read a body send
/// their good body.
pub async fn with_odd_paths_and_queries(router: &Router, calls: &[Call], key: &str) -> Vec<Answer> {
    let authorization = format!("Bearer {key}");
    let long = "a".repeat(VERY_LONG_IN_AN_ADDRESS);
    let last_parts = [
        ("a very long last part", long.clone()),
        ("a last part that is not UTF-8", "%ff%fe".to_owned()),
        ("a last part of one space", "%20".to_owned()),
        ("a last part with a slash in it", "a%2Fb".to_owned()),
        ("a last part of dots", "..".to_owned()),
    ];
    let queries = [
        ("limit is a word", "limit=many".to_owned()),
        ("limit is negative", "limit=-1".to_owned()),
        ("limit is zero", "limit=0".to_owned()),
        (
            "limit is a huge number",
            "limit=99999999999999999999999999".to_owned(),
        ),
        ("after is very long", format!("after={long}")),
        ("after is not UTF-8", "after=%ff".to_owned()),
        ("after is empty", "after=".to_owned()),
        ("a query nobody reads", "notAQuery=1".to_owned()),
        ("limit twice", "limit=1&limit=2".to_owned()),
    ];
    let mut answers = Vec::new();
    for call in calls {
        let body = || call.body.as_ref().map(|body| body.to_string().into_bytes());
        let (before_last, _) = call.path.rsplit_once('/').unwrap_or(("", call.path));
        let mut paths = Vec::new();
        for (what, last) in &last_parts {
            paths.push((*what, format!("{before_last}/{last}")));
        }
        for (what, query) in &queries {
            paths.push((*what, format!("{}?{query}", call.path)));
        }
        for (what, path) in paths {
            let status = status_of(router, call.method, &path, Some(&authorization), body()).await;
            answers.push(Answer {
                call: format!("{} {}, {what}", call.method, call.path),
                status,
            });
        }
    }
    answers
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use axum::Json;
    use axum::routing::post;

    use super::*;

    /// Whether one of `bodies` is called `what`.
    fn has(bodies: &[(String, Vec<u8>)], what: &str) -> bool {
        bodies.iter().any(|(name, _)| name == what)
    }

    #[test]
    fn the_wrong_bodies_cover_every_member_and_every_old_name() {
        let good = json!({"address": "https://a.example", "ids": ["a"], "offer": {"x": 1}});
        let bodies = wrong_bodies(&good, &[("address", "providerAddress")]);
        for what in [
            "an empty body",
            "a member the route does not know",
            "address under its old name providerAddress",
            "address is a number",
            "address is empty",
            "address is very long",
            "ids is a text",
            "ids holds an empty text",
            "offer is a list",
        ] {
            assert!(has(&bodies, what), "{what}");
        }
        let (_, renamed) = bodies
            .iter()
            .find(|(what, _)| what.contains("old name"))
            .unwrap();
        let renamed: Value = serde_json::from_slice(renamed).unwrap();
        assert_eq!(renamed["providerAddress"], "https://a.example");
        assert!(renamed.get("address").is_none());
    }

    #[test]
    fn only_the_statuses_not_allowed_are_listed() {
        let answers = [
            Answer {
                call: "GET /a".to_owned(),
                status: 400,
            },
            Answer {
                call: "GET /b".to_owned(),
                status: 500,
            },
        ];
        assert_eq!(statuses_other_than(&[400], &answers), "GET /b: 500\n");
        assert_eq!(statuses_other_than(&[400, 500], &answers), "");
    }

    /// A router that takes any JSON body and needs no key: every call
    /// is answered 200, so the checks have something to find.
    #[tokio::test]
    async fn a_router_that_accepts_everything_is_found_out() {
        let router = Router::new().route("/v1/things", post(|Json(_): Json<Value>| async { "ok" }));
        let calls = [Call::post("/v1/things", json!({"name": "a"}))];
        let answers = without_the_key(&router, &calls, "key").await;
        assert_eq!(answers.len(), 6);
        assert!(!statuses_other_than(&[401], &answers).is_empty());
        let answers = with_a_wrong_body(&router, &calls, "key").await;
        let wrong = statuses_other_than(&[400], &answers);
        assert!(
            wrong.contains("POST /v1/things, name is a number: 200"),
            "{wrong}"
        );
        // axum itself refuses what is not JSON
        assert!(!wrong.contains("a body that is not JSON"), "{wrong}");
    }
}
