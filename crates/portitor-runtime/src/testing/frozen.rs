// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The tools of the freeze guard: the tests named `frozen_interface` in
//! the three binaries' crates.
//!
//! From 0.13 the HTTP routes and the environment variables of the three
//! binaries are frozen (`docs/STABILITY.md`). That file holds the frozen
//! lists, each as a code block under a heading. The guard compares each
//! list with what the code has:
//!
//! - routes: the `OpenAPI` description ([`openapi_routes`]) and the real
//!   router ([`is_served`]);
//! - settings: every `"PORTITOR_..."` and `"OTEL_..."` text in the
//!   sources ([`settings_in_code`]), so nobody has to remember to keep a
//!   second list up to date.
//!
//! What the guard does not see, so a reviewer still has to:
//!
//! - It checks that a frozen setting's name is in the sources, not that
//!   the setting is still read. A name that stays only in an error text
//!   passes.
//! - A name that is not written whole between double quotes on one line
//!   (built with `concat!` or `format!`), or that follows an escaped
//!   quote (`\"`) on its line, is not found.
//! - The public routes are compared with the hand-written
//!   `docs/api/openapi.json`. A new public route that is added to the
//!   router but not to that file is not noticed; only the management
//!   descriptions are generated from the handlers.
//! - Sources are read up to their unit tests (`#[cfg(test)]` on a
//!   `mod`); files named `tests.rs` are left out.
//!
//! The functions here return an error text instead of panicking; the
//! tests print it.

use std::collections::BTreeSet;
use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt as _;

/// A set of names, sorted, so that error texts list them in order.
pub type Names = BTreeSet<String>;

/// Reads the file `path` of the repository (for example
/// `docs/STABILITY.md`). `manifest_dir` is the calling crate's folder,
/// `env!("CARGO_MANIFEST_DIR")`; every crate is two folders below the
/// repository's root.
///
/// # Errors
/// The file cannot be read.
pub fn repository_file(manifest_dir: &str, path: &str) -> Result<String, String> {
    let full = Path::new(manifest_dir).join("../..").join(path);
    std::fs::read_to_string(&full)
        .map_err(|error| format!("cannot read {}: {error}", full.display()))
}

/// The lines of the code block under `heading` in the Markdown `text`,
/// without empty lines. `heading` is the whole heading line, for example
/// `### portitor-connector, public port`. The code block is the first
/// one fenced with three backticks after the heading and before the next
/// heading.
///
/// # Errors
/// The heading is not there, or it has no code block.
pub fn list_under(text: &str, heading: &str) -> Result<Names, String> {
    let mut lines = text.lines();
    // go to the heading
    if !lines.any(|line| line.trim_end() == heading) {
        return Err(format!("docs/STABILITY.md has no heading \"{heading}\""));
    }
    // go to the start of its code block
    loop {
        match lines.next() {
            Some(line) if line.starts_with("```") => break,
            Some(line) if !line.starts_with('#') => {}
            _ => {
                return Err(format!(
                    "docs/STABILITY.md: no code block under \"{heading}\""
                ));
            }
        }
    }
    // take the lines up to the end of the code block
    let mut names = Names::new();
    for line in lines {
        if line.starts_with("```") {
            break;
        }
        let line = line.trim();
        if !line.is_empty() {
            names.insert(line.to_owned());
        }
    }
    Ok(names)
}

/// The routes an `OpenAPI` description lists, each as `METHOD path`, for
/// example `GET /v1/credentials/{id}`.
///
/// A path that has its own `servers` entry sits under a URL the operator
/// sets (the identity's credential service URL, the issuer's issuer
/// service URL). Its route starts with that entry's `url`, which the
/// descriptions write as a variable: `POST
/// {credentialService}/presentations/query`.
///
/// # Errors
/// The text is not JSON, or has no `paths`.
pub fn openapi_routes(openapi_json: &str) -> Result<Names, String> {
    let description: serde_json::Value =
        serde_json::from_str(openapi_json).map_err(|error| format!("not JSON: {error}"))?;
    let Some(paths) = description["paths"].as_object() else {
        return Err("the OpenAPI description has no paths".to_owned());
    };
    let mut routes = Names::new();
    for (path, item) in paths {
        // "" when the path has no `servers` entry of its own
        let prefix = item["servers"][0]["url"].as_str().unwrap_or_default();
        for method in ["get", "post", "put", "patch", "delete"] {
            if item.get(method).is_some() {
                routes.insert(format!("{} {prefix}{path}", method.to_uppercase()));
            }
        }
    }
    Ok(routes)
}

/// Whether `router` has the route `route` (`METHOD path`). Each `{name}`
/// in the path is asked for with the value `x`.
///
/// How: the request is sent without a token or a body, so most routes
/// refuse it (401, 400, their own 404). That is fine. Only two answers
/// mean "no such route": 405 (the path exists, not with this method) and
/// the answer of a fallback added here, 418, which axum gives only when
/// no path matches.
///
/// # Errors
/// `route` is not `METHOD path`, or the router fails.
pub async fn is_served(router: &Router, route: &str) -> Result<bool, String> {
    let Some((method, path)) = route.split_once(' ') else {
        return Err(format!("\"{route}\" is not \"METHOD path\""));
    };
    let request = Request::builder()
        .method(method)
        .uri(with_placeholders_filled(path))
        .body(Body::empty())
        .map_err(|error| format!("{route}: {error}"))?;
    let no_such_path = || async { StatusCode::IM_A_TEAPOT };
    let response = router
        .clone()
        .fallback(no_such_path)
        .oneshot(request)
        .await
        .map_err(|error| format!("{route}: {error}"))?;
    let status = response.status();
    Ok(status != StatusCode::IM_A_TEAPOT && status != StatusCode::METHOD_NOT_ALLOWED)
}

/// `path` with every `{name}` replaced by `x`:
/// `/v1/credentials/{id}/revoke` becomes `/v1/credentials/x/revoke`.
fn with_placeholders_filled(path: &str) -> String {
    let mut filled = String::new();
    let mut inside_braces = false;
    for character in path.chars() {
        match character {
            '{' => {
                inside_braces = true;
                filled.push('x');
            }
            '}' => inside_braces = false,
            _ if inside_braces => {}
            _ => filled.push(character),
        }
    }
    filled
}

/// The environment variables the code under `source_folders` names: every
/// text literal that is exactly `"PORTITOR_..."` or `"OTEL_..."` (capital
/// letters, digits and `_`). All Rust files below each folder are read.
///
/// Left out, because they are not what a running binary reads: comment
/// lines, files named `tests.rs`, the test support code (a file
/// `testing.rs` or a folder `testing`, built only for tests with the
/// `test-support` feature), and in each file its unit tests: the
/// `mod` that follows a `#[cfg(test)]` line. A module written in the
/// file (`mod tests {`) is taken to run to the end of the file; one in a
/// file of its own (`mod tests;`) is that one line. Any other item
/// marked `#[cfg(test)]` is read like the rest: the code after it is
/// what the binary runs.
///
/// # Errors
/// A folder or a file cannot be read.
pub fn settings_in_code(source_folders: &[&Path]) -> Result<Names, String> {
    let mut names = Names::new();
    for folder in source_folders {
        add_settings_of_folder(folder, &mut names)?;
    }
    Ok(names)
}

fn add_settings_of_folder(folder: &Path, names: &mut Names) -> Result<(), String> {
    let entries = std::fs::read_dir(folder)
        .map_err(|error| format!("cannot read {}: {error}", folder.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|error| format!("cannot read {}: {error}", folder.display()))?
            .path();
        let name = path.file_name().unwrap_or_default();
        if name == "testing" || name == "testing.rs" {
            // test support code: never part of a binary
            continue;
        }
        if path.is_dir() {
            add_settings_of_folder(&path, names)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") && name != "tests.rs"
        {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            add_settings_of_file(&text, names);
        }
    }
    Ok(())
}

fn add_settings_of_file(text: &str, names: &mut Names) {
    // whether the last attribute seen was `#[cfg(test)]`, with only
    // other attributes after it
    let mut for_tests = false;
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with("//") || line.is_empty() {
            continue;
        }
        if line.starts_with("#[cfg(test)]") {
            for_tests = true;
        } else if for_tests && is_module_start(line) {
            // the unit tests: the rest of the file is no release code.
            // `#[cfg(test)]` on a single item (a function, a `use`) does
            // not end the reading
            return;
        } else if !line.starts_with("#[") {
            for_tests = false;
        }
        // the pieces between double quotes are at the odd positions:
        // `f("A", "B")` splits into `f(`, `A`, `, `, `B`, `)`
        for (position, piece) in line.split('"').enumerate() {
            if position % 2 == 1 && is_setting_name(piece) {
                names.insert(piece.to_owned());
            }
        }
    }
}

/// Whether `line` starts a module written out here (`mod tests {`,
/// `pub(crate) mod x {`).
fn is_module_start(line: &str) -> bool {
    let line = match line.strip_prefix("pub") {
        Some(rest) => rest
            .trim_start_matches(|character| character != ' ')
            .trim_start(),
        None => line,
    };
    // `mod more_tests;` names another file and does not end this one
    line.starts_with("mod ") && line.trim_end().ends_with('{')
}

/// Whether `text` looks like one of our environment variables: ours
/// (`PORTITOR_`), OpenTelemetry's (`OTEL_`), or the AWS SDK names the
/// connector reads for object storage (`AWS_`, ADR 0027).
fn is_setting_name(text: &str) -> bool {
    let rest = text
        .strip_prefix("PORTITOR_")
        .or_else(|| text.strip_prefix("OTEL_"))
        .or_else(|| text.strip_prefix("AWS_"));
    match rest {
        Some(rest) => {
            !rest.is_empty()
                && rest.chars().all(|character| {
                    character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
                })
        }
        None => false,
    }
}

/// Compares a frozen list with what the code has. `what` names the list
/// in the error text, for example `settings of portitor-identity`.
///
/// # Errors
/// The two differ. The text names what is gone from the code and what is
/// new in the code, and says what to do.
pub fn compare(what: &str, frozen: &Names, in_code: &Names) -> Result<(), String> {
    let gone: Vec<&String> = frozen.difference(in_code).collect();
    let new: Vec<&String> = in_code.difference(frozen).collect();
    if gone.is_empty() && new.is_empty() {
        return Ok(());
    }
    let mut parts = vec![format!(
        "The {what} differ from the frozen list in docs/STABILITY.md."
    )];
    if !gone.is_empty() {
        parts.push(format!("Frozen, but gone from the code: {gone:?}"));
        parts.push(
            "  Removing or renaming a frozen name breaks users of 1.x. Put it back. \
             Only a new major version may remove it."
                .to_owned(),
        );
    }
    if !new.is_empty() {
        parts.push(format!("In the code, but not frozen: {new:?}"));
        parts.push(
            "  Adding is allowed. Add each name to its list in docs/STABILITY.md, so that a \
             reviewer sees that the stable interface grows. From then on it cannot be removed."
                .to_owned(),
        );
    }
    Err(parts.join("\n"))
}

/// The whole settings check of one binary: its frozen settings in
/// `stability` (the text of `docs/STABILITY.md`: the lists "Settings of
/// every binary" and "Settings of `binary`") are the settings its code
/// names under `source_folders`, apart from `not_frozen` (names the code
/// holds that are no settings of a release build).
///
/// # Errors
/// A list is missing, a folder cannot be read, or the two differ.
pub fn check_settings(
    stability: &str,
    binary: &str,
    source_folders: &[&Path],
    not_frozen: &[&str],
) -> Result<(), String> {
    let mut frozen = list_under(stability, "### Settings of every binary")?;
    frozen.append(&mut list_under(
        stability,
        &format!("### Settings of {binary}"),
    )?);
    let mut in_code = settings_in_code(source_folders)?;
    for name in not_frozen {
        if !in_code.remove(*name) {
            return Err(format!(
                "{name} is listed as not frozen, but the code of {binary} no longer names it: \
                 take it off that list in the test"
            ));
        }
    }
    compare(&format!("settings of {binary}"), &frozen, &in_code)
}

/// The whole route check of one port: the `frozen` routes are the
/// `documented` ones, and `router` serves each of them. `what` names the
/// port in the error text. `url_variables` gives the path for each
/// variable a route starts with, as (`{credentialService}`, `/cs`).
///
/// # Errors
/// The lists differ, a frozen route is not served, or the router answers
/// every path, so that nothing can be told
/// ([`answered_under_a_wrong_name`]).
pub async fn check_routes(
    what: &str,
    frozen: &Names,
    documented: &Names,
    router: &Router,
    url_variables: &[(&str, &str)],
) -> Result<(), String> {
    compare(what, frozen, documented)?;
    for route in frozen {
        let mut on_this_router = route.clone();
        for (variable, path) in url_variables {
            on_this_router = on_this_router.replace(variable, path);
        }
        if !is_served(router, &on_this_router).await? {
            return Err(format!(
                "{what}: the frozen route \"{route}\" is in docs/STABILITY.md and in the \
                 OpenAPI description, but the router does not serve it. Removing or renaming \
                 a frozen route breaks users of 1.x: put it back."
            ));
        }
    }
    // the check above means something only if the router does not
    // answer every path
    let answered = answered_under_a_wrong_name(router, frozen, url_variables).await?;
    if !answered.is_empty() {
        return Err(format!(
            "{what}: the router also answers paths that no route has: {answered:?}. \
             On such a router this check cannot see a removed or renamed route."
        ));
    }
    Ok(())
}

/// A check of the guard itself on a real router: the `routes` that
/// `router` also answers under a name no route has. Each route is asked
/// for with `-renamed` added to the last fixed part of its path
/// (`GET /v1/things/{id}` as `GET /v1/things-renamed/x`).
///
/// The result must be empty. A router that answers every path (a
/// catch-all route, or a nested router with its own "not found" answer)
/// would make [`is_served`] say yes to anything, and the freeze guard
/// would pass whatever route is removed or renamed.
///
/// # Errors
/// See [`is_served`].
pub async fn answered_under_a_wrong_name(
    router: &Router,
    frozen: &Names,
    url_variables: &[(&str, &str)],
) -> Result<Names, String> {
    let mut answered = Names::new();
    for route in frozen {
        let mut on_this_router = route.clone();
        for (variable, path) in url_variables {
            on_this_router = on_this_router.replace(variable, path);
        }
        let Some(renamed) = with_last_fixed_part_renamed(&on_this_router) else {
            continue;
        };
        if is_served(router, &renamed).await? {
            answered.insert(format!("{route} (asked as {renamed})"));
        }
    }
    Ok(answered)
}

/// `route` (`METHOD path`) with `-renamed` added to the last part of its
/// path that is not a `{name}`; `None` when the path has no such part.
fn with_last_fixed_part_renamed(route: &str) -> Option<String> {
    let mut parts: Vec<String> = route.split('/').map(str::to_owned).collect();
    // `parts[0]` is the method (and a space); the path's parts follow
    let last_fixed = parts
        .iter()
        .rposition(|part| !part.is_empty() && !part.starts_with('{'))?;
    if last_fixed == 0 {
        return None;
    }
    parts[last_fixed].push_str("-renamed");
    Some(parts.join("/"))
}

/// The frozen routes of the management port of `binary`: the list every
/// binary shares and its own.
///
/// # Errors
/// A list is missing.
pub fn management_routes(stability: &str, binary: &str) -> Result<Names, String> {
    let mut frozen = list_under(stability, "### Every binary, management port")?;
    frozen.append(&mut list_under(
        stability,
        &format!("### {binary}, management port"),
    )?);
    Ok(frozen)
}

/// What the management port of a binary serves, as far as it is written
/// down: the routes of its generated `OpenAPI` description (which holds
/// the health checks too), and the two open routes the description leaves
/// out, itself and the old `GET /health`.
///
/// # Errors
/// See [`openapi_routes`].
pub fn documented_management_routes(openapi_json: &str) -> Result<Names, String> {
    let mut routes = openapi_routes(openapi_json)?;
    routes.insert("GET /v1/openapi.json".to_owned());
    routes.insert("GET /health".to_owned());
    Ok(routes)
}

/// The routes of a management `OpenAPI` description that need the API
/// key (they have a `security` entry) but do not list HTTP 403 among
/// their answers. 403 is reserved for tokens with limited rights in 1.x
/// (ADR 0025), so every such route documents it from 1.0 on.
///
/// # Errors
/// The text is not JSON, or has no `paths`.
pub fn protected_routes_without_403(openapi_json: &str) -> Result<Names, String> {
    let description: serde_json::Value =
        serde_json::from_str(openapi_json).map_err(|error| format!("not JSON: {error}"))?;
    let Some(paths) = description["paths"].as_object() else {
        return Err("the OpenAPI description has no paths".to_owned());
    };
    let mut missing = Names::new();
    for (path, item) in paths {
        for method in ["get", "post", "put", "patch", "delete"] {
            let Some(operation) = item.get(method) else {
                continue;
            };
            let needs_key = operation.get("security").is_some();
            let documents_403 = operation["responses"].get("403").is_some();
            if needs_key && !documents_403 {
                missing.insert(format!("{} {path}", method.to_uppercase()));
            }
        }
    }
    Ok(missing)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Names {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_list_is_the_code_block_under_its_heading() {
        let text = "# T\n\n### One\n\nSome words.\n\n```text\nGET /a\n\nPOST /b\n```\n\n### Two\n\nNo block.\n\n### Three\n```\nX\n```\n";
        assert_eq!(
            list_under(text, "### One").unwrap(),
            names(&["GET /a", "POST /b"])
        );
        assert_eq!(list_under(text, "### Three").unwrap(), names(&["X"]));
        // the block of the next heading is not taken
        assert!(
            list_under(text, "### Two")
                .unwrap_err()
                .contains("no code block")
        );
        assert!(
            list_under(text, "### Four")
                .unwrap_err()
                .contains("no heading")
        );
    }

    #[test]
    fn routes_of_a_description() {
        let description = r#"{"paths": {
            "/v1/things": {"get": {}, "post": {}, "parameters": []},
            "/query": {"servers": [{"url": "{service}"}], "post": {}}
        }}"#;
        assert_eq!(
            openapi_routes(description).unwrap(),
            names(&["GET /v1/things", "POST /v1/things", "POST {service}/query"])
        );
        assert!(openapi_routes("{}").is_err());
    }

    #[tokio::test]
    async fn a_route_is_served_whatever_it_answers() {
        use axum::routing::{get, post};
        let router = Router::new()
            .route("/things/{id}", get(|| async { StatusCode::NOT_FOUND }))
            .route("/things", post(|| async { StatusCode::UNAUTHORIZED }));
        assert!(is_served(&router, "GET /things/{id}").await.unwrap());
        assert!(is_served(&router, "POST /things").await.unwrap());
        // the path exists, the method does not
        assert!(!is_served(&router, "GET /things").await.unwrap());
        assert!(!is_served(&router, "GET /other").await.unwrap());
        assert!(is_served(&router, "things").await.is_err());
    }

    #[test]
    fn settings_are_the_quoted_names_before_the_unit_tests() {
        let mut found = Names::new();
        let file = "let a = var(\"PORTITOR_A\"); // \"PORTITOR_IN_A_COMMENT_AT_THE_END\"\n\
                    // \"PORTITOR_COMMENT\"\n\
                    let b = (\"OTEL_B_2\", \"PORTITOR_C is required\", \"portitor\");\n\
                    #[cfg(test)]\n\
                    fn only_for_tests() {}\n\
                    let d = var(\"PORTITOR_D\");\n\
                    #[cfg(test)]\n\
                    #[allow(clippy::unwrap_used)]\n\
                    mod tests {\n\
                    let c = \"PORTITOR_TEST_ONLY\";\n";
        add_settings_of_file(file, &mut found);
        // a name in a comment at the end of a code line is found too:
        // rare, and it errs on the side of asking. `#[cfg(test)]` on one
        // item does not end the reading; on the test module it does
        assert_eq!(
            found,
            names(&[
                "OTEL_B_2",
                "PORTITOR_A",
                "PORTITOR_D",
                "PORTITOR_IN_A_COMMENT_AT_THE_END"
            ])
        );
    }

    #[test]
    fn a_protected_route_without_403_is_found() {
        let description = r#"{"paths": {
            "/open": {"get": {"responses": {"200": {}}}},
            "/a": {"get": {"security": [{"api_key": []}], "responses": {"200": {}, "403": {}}}},
            "/b": {"post": {"security": [{"api_key": []}], "responses": {"200": {}}}}
        }}"#;
        assert_eq!(
            protected_routes_without_403(description).unwrap(),
            names(&["POST /b"])
        );
    }

    #[test]
    fn a_difference_says_what_to_do() {
        let frozen = names(&["A", "B"]);
        assert_eq!(compare("things", &frozen, &frozen), Ok(()));
        let error = compare("things", &frozen, &names(&["B", "C"])).unwrap_err();
        assert!(error.contains("gone from the code: [\"A\"]"), "{error}");
        assert!(error.contains("not frozen: [\"C\"]"), "{error}");
    }

    /// A `#[cfg(test)]` line that marks something other than the unit
    /// tests does not end the reading: what follows it is code the
    /// binary runs. (The connector's `harvest/catalog.rs` has such a line
    /// near its top.)
    #[test]
    fn settings_after_a_test_only_item_are_still_found() {
        let mut found = Names::new();
        let file = "#[cfg(test)]\n\
                    impl Fake for Thing {\n\
                    }\n\
                    let a = var(\"PORTITOR_AFTER_A_TEST_ONLY_IMPL\");\n\
                    #[cfg(test)]\n\
                    mod more_tests;\n\
                    let b = var(\"PORTITOR_AFTER_A_TEST_FILE\");\n\
                    #[cfg(test)] // the tests\n\
                    #[allow(clippy::unwrap_used)]\n\
                    mod tests {\n\
                    let c = \"PORTITOR_TEST_ONLY\";\n\
                    }\n";
        add_settings_of_file(file, &mut found);
        assert_eq!(
            found,
            names(&[
                "PORTITOR_AFTER_A_TEST_FILE",
                "PORTITOR_AFTER_A_TEST_ONLY_IMPL"
            ])
        );
    }

    /// Two source folders as a binary has them: its own crate, with a
    /// `tests.rs`, test support code (`testing.rs`, `testing/`) and a
    /// folder inside, and a shared crate.
    fn source_folders() -> crate::testing::TempDir {
        let dir = crate::testing::TempDir::new("frozen");
        std::fs::create_dir_all(dir.path().join("own/src/auth")).unwrap();
        std::fs::create_dir_all(dir.path().join("shared/src")).unwrap();
        let _ = dir.write("own/src/main.rs", "let a = var(\"PORTITOR_A\");\n");
        let _ = dir.write("own/src/auth/sts.rs", "let b = var(\"PORTITOR_B\");\n");
        let _ = dir.write("own/src/tests.rs", "let t = var(\"PORTITOR_TEST\");\n");
        std::fs::create_dir_all(dir.path().join("own/src/testing")).unwrap();
        let _ = dir.write(
            "own/src/testing.rs",
            "let t = var(\"PORTITOR_TEST_SUPPORT\");\n",
        );
        let _ = dir.write(
            "own/src/testing/db.rs",
            "let t = var(\"PORTITOR_TEST_DB\");\n",
        );
        let _ = dir.write("own/src/notes.md", "\"PORTITOR_NOT_RUST\"\n");
        let _ = dir.write("shared/src/lib.rs", "let o = var(\"OTEL_C\");\n");
        dir
    }

    const STABILITY: &str = "### Settings of every binary\n\n```text\nOTEL_C\n```\n\n\
                             ### Settings of own\n\n```text\nPORTITOR_A\nPORTITOR_B\n```\n";

    /// The settings check as a binary's test calls it, with each change
    /// someone could make to the list or to the code.
    #[test]
    fn the_settings_check_fails_for_every_kind_of_difference() {
        let dir = source_folders();
        let (own, shared) = (dir.path().join("own/src"), dir.path().join("shared/src"));
        let folders = [own.as_path(), shared.as_path()];
        let check = |stability: &str, not_frozen: &[&str]| {
            check_settings(stability, "own", &folders, not_frozen)
        };
        assert_eq!(check(STABILITY, &[]), Ok(()));

        // a line removed from the list: the code still has the name
        let removed = STABILITY.replace("PORTITOR_B\n", "");
        let error = check(&removed, &[]).unwrap_err();
        assert!(error.contains("not frozen: [\"PORTITOR_B\"]"), "{error}");
        // the same line removed from the shared list
        let removed = STABILITY.replace("OTEL_C\n", "");
        let error = check(&removed, &[]).unwrap_err();
        assert!(error.contains("not frozen: [\"OTEL_C\"]"), "{error}");
        // a line added to the list that the code does not have
        let added = STABILITY.replace("PORTITOR_B\n", "PORTITOR_B\nPORTITOR_NEW\n");
        let error = check(&added, &[]).unwrap_err();
        assert!(
            error.contains("gone from the code: [\"PORTITOR_NEW\"]"),
            "{error}"
        );
        // a name renamed in the list
        let renamed = STABILITY.replace("PORTITOR_A\n", "PORTITOR_A2\n");
        let error = check(&renamed, &[]).unwrap_err();
        assert!(
            error.contains("gone from the code: [\"PORTITOR_A2\"]"),
            "{error}"
        );
        assert!(error.contains("not frozen: [\"PORTITOR_A\"]"), "{error}");
        // a list that is not there at all
        let error = check("### Settings of every binary\n```\nOTEL_C\n```\n", &[]).unwrap_err();
        assert!(
            error.contains("no heading \"### Settings of own\""),
            "{error}"
        );

        // a name set aside as not frozen is taken out of the code's names
        let set_aside = check(&STABILITY.replace("PORTITOR_B\n", ""), &["PORTITOR_B"]);
        assert_eq!(set_aside, Ok(()));
        // and must still be in the code
        let error = check(STABILITY, &["PORTITOR_GONE"]).unwrap_err();
        assert!(
            error.contains("PORTITOR_GONE is listed as not frozen"),
            "{error}"
        );

        // the code changes: a new setting, and a renamed one
        let _ = dir.write("own/src/new.rs", "let n = var(\"PORTITOR_N\");\n");
        let error = check(STABILITY, &[]).unwrap_err();
        assert!(error.contains("not frozen: [\"PORTITOR_N\"]"), "{error}");
        std::fs::remove_file(own.join("new.rs")).unwrap();
        let _ = dir.write("own/src/auth/sts.rs", "let b = var(\"PORTITOR_B2\");\n");
        let error = check(STABILITY, &[]).unwrap_err();
        assert!(
            error.contains("gone from the code: [\"PORTITOR_B\"]"),
            "{error}"
        );

        // a folder that is not there is an error, not an empty list
        let missing = dir.path().join("no-such-crate/src");
        let error = check_settings(STABILITY, "own", &[missing.as_path()], &[]).unwrap_err();
        assert!(error.starts_with("cannot read "), "{error}");
    }

    /// The route check as a binary's test calls it, with each change
    /// someone could make to the list, the description or the router.
    #[tokio::test]
    async fn the_route_check_fails_for_every_kind_of_difference() {
        use axum::routing::{get, post};
        let frozen = names(&["GET /v1/things/{id}", "POST {service}/query"]);
        let router = || {
            Router::new()
                .route(
                    "/v1/things/{id}",
                    get(|| async { StatusCode::UNAUTHORIZED }),
                )
                .route("/svc/query", post(|| async { StatusCode::BAD_REQUEST }))
        };
        let variables = [("{service}", "/svc")];
        let check = |frozen: Names, documented: Names, router: Router| async move {
            check_routes("routes", &frozen, &documented, &router, &variables).await
        };
        assert_eq!(
            check(frozen.clone(), frozen.clone(), router()).await,
            Ok(())
        );

        // a route removed from the list, and one added to it
        let shorter = names(&["GET /v1/things/{id}"]);
        let error = check(shorter, frozen.clone(), router()).await.unwrap_err();
        assert!(
            error.contains("not frozen: [\"POST {service}/query\"]"),
            "{error}"
        );
        let mut longer = frozen.clone();
        longer.insert("DELETE /v1/things/{id}".to_owned());
        let error = check(longer.clone(), frozen.clone(), router())
            .await
            .unwrap_err();
        assert!(
            error.contains("gone from the code: [\"DELETE /v1/things/{id}\"]"),
            "{error}"
        );
        // listed and documented, but the router does not have the method
        let error = check(longer.clone(), longer, router()).await.unwrap_err();
        assert!(error.contains("\"DELETE /v1/things/{id}\""), "{error}");
        assert!(error.contains("the router does not serve it"), "{error}");

        // a route renamed in the router only
        let renamed = Router::new()
            .route("/v1/items/{id}", get(|| async { StatusCode::UNAUTHORIZED }))
            .route("/svc/query", post(|| async { StatusCode::BAD_REQUEST }));
        let error = check(frozen.clone(), frozen.clone(), renamed)
            .await
            .unwrap_err();
        assert!(error.contains("\"GET /v1/things/{id}\""), "{error}");
        // a route moved to another path of the service URL
        let moved = Router::new()
            .route(
                "/v1/things/{id}",
                get(|| async { StatusCode::UNAUTHORIZED }),
            )
            .route("/other/query", post(|| async { StatusCode::BAD_REQUEST }));
        let error = check(frozen.clone(), frozen.clone(), moved)
            .await
            .unwrap_err();
        assert!(error.contains("\"POST {service}/query\""), "{error}");
        // a router that kept the path and lost the method
        let other_method = Router::new()
            .route(
                "/v1/things/{id}",
                post(|| async { StatusCode::UNAUTHORIZED }),
            )
            .route("/svc/query", post(|| async { StatusCode::BAD_REQUEST }));
        let error = check(frozen.clone(), frozen.clone(), other_method)
            .await
            .unwrap_err();
        assert!(error.contains("\"GET /v1/things/{id}\""), "{error}");
    }

    #[tokio::test]
    async fn a_router_that_answers_every_path_is_found_out() {
        use axum::routing::get;
        let routes = names(&["GET /v1/things/{id}", "GET {service}/list", "GET /"]);
        let variables = [("{service}", "/svc")];
        let exact = Router::new()
            .route("/v1/things/{id}", get(|| async { "" }))
            .route("/svc/list", get(|| async { "" }))
            .route("/", get(|| async { "" }));
        let answered = answered_under_a_wrong_name(&exact, &routes, &variables);
        assert_eq!(answered.await.unwrap(), Names::new());

        // a nested router with its own "not found" answers every path
        // below `/svc`
        let service = Router::new()
            .route("/list", get(|| async { "" }))
            .fallback(|| async { StatusCode::NOT_FOUND });
        let nested = Router::new()
            .route("/v1/things/{id}", get(|| async { "" }))
            .nest("/svc", service);
        let answered = answered_under_a_wrong_name(&nested, &routes, &variables);
        assert_eq!(
            answered.await.unwrap(),
            names(&["GET {service}/list (asked as GET /svc/list-renamed)"])
        );
        // a catch-all route answers every path
        let catch_all = Router::new().route("/{*path}", get(|| async { "" }));
        let answered = answered_under_a_wrong_name(&catch_all, &routes, &variables);
        assert_eq!(answered.await.unwrap().len(), 2);
    }

    /// The management lists: the shared one and the binary's own, and    /// The management lists: the shared one and the binary's own, and
    /// the two open routes a description leaves out.
    #[test]
    fn management_routes_are_the_shared_list_and_the_binarys_own() {
        let stability = "### Every binary, management port\n```\nGET /health\n```\n\
                         ### own, management port\n```\nGET /v1/things\n```\n";
        assert_eq!(
            management_routes(stability, "own").unwrap(),
            names(&["GET /health", "GET /v1/things"])
        );
        assert!(management_routes(stability, "other").is_err());
        let documented =
            documented_management_routes(r#"{"paths": {"/v1/things": {"get": {}}}}"#).unwrap();
        assert_eq!(
            documented,
            names(&["GET /health", "GET /v1/openapi.json", "GET /v1/things"])
        );
    }
}
