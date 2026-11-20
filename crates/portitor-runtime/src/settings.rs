// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Helpers for reading settings from environment variables, shared by
//! the connector, the identity component and the issuer.
//!
//! One rule for every setting of the three binaries, from 0.13: spaces
//! around a value are dropped, and an empty value means "not set". So a
//! required setting that is empty is missing, and an optional one takes
//! its default. Read a setting with [`text`] (or a helper built on it),
//! never with `std::env::var` directly, so that the rule holds
//! everywhere.
//!
//! The functions take the lookup as `var`: `std::env::var_os` in a
//! binary, a list of pairs in tests (`crate::testing::vars`).

use std::ffi::OsString;

/// The value of the setting `name`, without the spaces around it. `None`
/// when the setting is not set, or when its value is empty or only
/// spaces: an empty value means "not set".
pub fn text(var: impl Fn(&str) -> Option<OsString>, name: &str) -> Option<String> {
    let value = var(name)?;
    // `to_string_lossy`: a value that is not UTF-8 is read with its bad
    // bytes replaced, so it fails later as an invalid value, with its name
    let value = value.to_string_lossy();
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

/// [`text`] of the environment variable `name`.
#[must_use]
pub fn text_from_env(name: &str) -> Option<String> {
    text(|name| std::env::var_os(name), name)
}

/// Whether the switch `name` is on: `true` or `false`, in any case. Not
/// set (or empty) is off.
///
/// # Errors
/// Any other value, with what is wrong, written to follow the name:
/// `must be true or false, not "1"`. A value such as `1` or `yes` that
/// was silently read as off would leave the operator with a switch they
/// believe is on.
pub fn switch(var: impl Fn(&str) -> Option<OsString>, name: &str) -> Result<bool, String> {
    match text(var, name) {
        None => Ok(false),
        Some(value) if value.eq_ignore_ascii_case("true") => Ok(true),
        Some(value) if value.eq_ignore_ascii_case("false") => Ok(false),
        Some(value) => Err(format!("must be true or false, not {value:?}")),
    }
}

/// A setting that a release took away or renamed. A service that finds
/// it set refuses to start and says what to set instead
/// ([`refuse_removed`]): an old name that is silently ignored would
/// leave the operator with a default they did not choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemovedSetting {
    /// The old name.
    pub name: &'static str,
    /// What happened and what to do, written to follow the name: `was
    /// renamed in 0.13; set PORTITOR_NEW instead`.
    pub advice: &'static str,
}

impl std::fmt::Display for RemovedSetting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.name, self.advice)
    }
}

/// `PORTITOR_ALLOW_HTTP_CALLBACKS`, the name of `PORTITOR_ALLOW_HTTP`
/// before 0.13, in the three binaries. The setting allows plain `http://`
/// for every outgoing call, not only for callbacks.
pub const ALLOW_HTTP_CALLBACKS: RemovedSetting = RemovedSetting {
    name: "PORTITOR_ALLOW_HTTP_CALLBACKS",
    advice: "was renamed in 0.13; set PORTITOR_ALLOW_HTTP instead",
};

/// Looks for a removed setting that is still set (to a value that is not
/// empty). Each binary calls this first when it reads its settings.
///
/// # Errors
/// The first of `removed` that is set; show it to the operator and do
/// not start.
pub fn refuse_removed(
    var: impl Fn(&str) -> Option<OsString>,
    removed: &[RemovedSetting],
) -> Result<(), RemovedSetting> {
    for setting in removed {
        if text(&var, setting.name).is_some() {
            return Err(*setting);
        }
    }
    Ok(())
}

/// The items of a comma-separated setting, trimmed, without empty ones:
/// `"a, b,,c "` is `["a", "b", "c"]`.
#[must_use]
pub fn comma_list(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    for item in text.split(',') {
        let item = item.trim();
        if !item.is_empty() {
            items.push(item.to_owned());
        }
    }
    items
}

/// The longest item a comma-separated setting may hold, in bytes.
///
/// The items are DIDs, DCP scopes, credential types and file paths. 4096
/// is `PATH_MAX` on Linux, so no file path that the system can open is
/// refused, and it is far above any DID, URL or scope in use. A longer
/// item is most likely a mistake (a missing comma, another separator, a
/// file's content instead of its name); without a limit it would only
/// fail later, as a long line in a log or an error.
pub const LIST_ITEM_MAX_BYTES: usize = 4096;

/// How much of an item that is too long an error shows, in characters.
const SHOWN_CHARACTERS: usize = 40;

/// The items of the comma-separated setting `name` ([`comma_list`]);
/// `None` when it is not set (or empty).
///
/// # Errors
/// What is wrong, written to follow the name:
/// - The setting is set but names nothing (only commas and spaces). One
///   rule for every list: such a value is most likely a mistake, and
///   whether it would mean "the default" or "nothing at all" differs
///   from list to list.
/// - An item is longer than [`LIST_ITEM_MAX_BYTES`].
pub fn list(
    var: impl Fn(&str) -> Option<OsString>,
    name: &str,
) -> Result<Option<Vec<String>>, String> {
    let Some(value) = text(var, name) else {
        return Ok(None);
    };
    let items = comma_list(&value);
    if items.is_empty() {
        return Err(format!(
            "names nothing ({value:?}): list at least one item, or leave it unset"
        ));
    }
    for (index, item) in items.iter().enumerate() {
        if item.len() > LIST_ITEM_MAX_BYTES {
            return Err(item_too_long(index, item));
        }
    }
    Ok(Some(items))
}

/// The error of [`list`] for an item over [`LIST_ITEM_MAX_BYTES`]. It
/// says which item (counted from 1) and shows only its start: the whole
/// item could fill the screen, or be a secret pasted by mistake.
fn item_too_long(index: usize, item: &str) -> String {
    let start: String = item.chars().take(SHOWN_CHARACTERS).collect();
    format!(
        "has an item that is too long: item {} (it starts with {start:?}) is {} bytes, and an item may be at most {LIST_ITEM_MAX_BYTES} bytes; separate the items with commas",
        index + 1,
        item.len()
    )
}

/// Reads a whole-number setting named `name`. `value` is the
/// variable's value, `None` when it is not set. The value is trimmed; an
/// empty value counts as not set. A setting not set is `default`.
///
/// # Errors
/// The value is not a whole number within `range`. The text names the
/// setting, its value and the range.
pub fn bounded_number(
    name: &str,
    value: Option<&str>,
    range: std::ops::RangeInclusive<u64>,
    default: u64,
) -> Result<u64, String> {
    let value = value.map(str::trim).unwrap_or_default();
    if value.is_empty() {
        return Ok(default);
    }
    match value.parse::<u64>() {
        Ok(number) if range.contains(&number) => Ok(number),
        _ => Err(format!(
            "{name} has an invalid value {value:?}: it must be a whole number from {} to {}",
            range.start(),
            range.end()
        )),
    }
}

/// The file paths of a comma-separated setting, read as [`comma_list`].
#[must_use]
pub fn path_list(text: &str) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    for item in comma_list(text) {
        paths.push(std::path::PathBuf::from(item));
    }
    paths
}

/// The path part of an `http(s)` URL, without a trailing `/`: `""` for
/// `https://id.example/`, `"/cs"` for `https://id.example/cs/`. `None`
/// if it is not an `http(s)` URL with a host, or has a query or fragment.
/// A component serves its DCP routes under the path of its public service
/// URL.
#[must_use]
pub fn base_path(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    if rest.contains(['?', '#']) {
        return None;
    }
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    if host.is_empty() {
        return None;
    }
    Some(format!("/{path}").trim_end_matches('/').to_owned())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_trimmed_and_an_empty_one_is_not_set() {
        let var = crate::testing::vars(&[("A", " a b "), ("EMPTY", ""), ("SPACES", " \t ")]);
        assert_eq!(text(&var, "A").as_deref(), Some("a b"));
        assert_eq!(text(&var, "EMPTY"), None);
        assert_eq!(text(&var, "SPACES"), None);
        assert_eq!(text(&var, "MISSING"), None);
    }

    #[test]
    fn a_switch_is_true_or_false_and_nothing_else() {
        let var = crate::testing::vars(&[
            ("ON", " TRUE "),
            ("ALSO_ON", "true"),
            ("OFF", "False"),
            ("EMPTY", ""),
            ("ONE", "1"),
            ("YES", "yes"),
            ("TYPO", "ture"),
        ]);
        assert_eq!(switch(&var, "ON"), Ok(true));
        assert_eq!(switch(&var, "ALSO_ON"), Ok(true));
        assert_eq!(switch(&var, "OFF"), Ok(false));
        assert_eq!(switch(&var, "EMPTY"), Ok(false));
        assert_eq!(switch(&var, "MISSING"), Ok(false));
        assert_eq!(
            switch(&var, "ONE"),
            Err("must be true or false, not \"1\"".to_owned())
        );
        assert!(switch(&var, "YES").is_err());
        assert!(switch(&var, "TYPO").is_err());
    }

    #[test]
    fn a_list_that_is_set_names_something() {
        let var = crate::testing::vars(&[("LIST", " a, ,b "), ("COMMAS", " , ,"), ("EMPTY", "")]);
        assert_eq!(
            list(&var, "LIST"),
            Ok(Some(vec!["a".to_owned(), "b".to_owned()]))
        );
        assert_eq!(list(&var, "EMPTY"), Ok(None));
        assert_eq!(list(&var, "MISSING"), Ok(None));
        let refused = list(&var, "COMMAS").unwrap_err();
        assert!(refused.starts_with("names nothing"), "{refused}");
    }

    #[test]
    fn a_list_item_may_be_as_long_as_the_limit_and_no_longer() {
        let longest = "a".repeat(LIST_ITEM_MAX_BYTES);
        let too_long = "b".repeat(LIST_ITEM_MAX_BYTES + 1);
        let var = crate::testing::vars(&[
            ("AT_LIMIT", &format!("short, {longest} ")),
            ("OVER", &format!("short,{too_long}")),
            // bytes are counted, not characters: `é` is two bytes
            ("OVER_IN_BYTES", &"é".repeat(LIST_ITEM_MAX_BYTES / 2 + 1)),
        ]);
        assert_eq!(
            list(&var, "AT_LIMIT"),
            Ok(Some(vec!["short".to_owned(), longest.clone()]))
        );
        let refused = list(&var, "OVER").unwrap_err();
        assert_eq!(
            refused,
            format!(
                "has an item that is too long: item 2 (it starts with {:?}) is 4097 bytes, and an item may be at most 4096 bytes; separate the items with commas",
                "b".repeat(40)
            )
        );
        // the error shows the start of the item, never all of it
        assert!(refused.len() < 250, "{refused}");
        let refused = list(&var, "OVER_IN_BYTES").unwrap_err();
        assert!(refused.contains("item 1 "), "{refused}");
        assert!(refused.contains("is 4098 bytes"), "{refused}");
    }

    #[test]
    fn a_removed_setting_that_is_set_is_refused_with_its_advice() {
        let removed = [ALLOW_HTTP_CALLBACKS];
        let unset = crate::testing::vars(&[("PORTITOR_ALLOW_HTTP", "true")]);
        assert_eq!(refuse_removed(&unset, &removed), Ok(()));
        // empty means not set, for an old name too
        let empty = crate::testing::vars(&[("PORTITOR_ALLOW_HTTP_CALLBACKS", " ")]);
        assert_eq!(refuse_removed(&empty, &removed), Ok(()));
        // any value refuses, also `false`: the operator must move to the
        // new name
        let set = crate::testing::vars(&[("PORTITOR_ALLOW_HTTP_CALLBACKS", "false")]);
        let refused = refuse_removed(&set, &removed).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "PORTITOR_ALLOW_HTTP_CALLBACKS was renamed in 0.13; set PORTITOR_ALLOW_HTTP instead"
        );
    }

    #[test]
    fn base_paths() {
        assert_eq!(base_path("https://id.example").as_deref(), Some(""));
        assert_eq!(base_path("https://id.example/").as_deref(), Some(""));
        assert_eq!(
            base_path("http://id.example:8090/cs/").as_deref(),
            Some("/cs")
        );
        assert_eq!(base_path("ftp://id.example"), None);
        assert_eq!(base_path("https:///cs"), None);
        assert_eq!(base_path("https://id.example/cs?x=1"), None);
        assert_eq!(base_path("https://id.example?x=1"), None);
        assert_eq!(base_path("https://id.example#x"), None);
    }

    #[test]
    fn bounded_numbers_are_trimmed_checked_and_default_when_empty() {
        let read = |value| bounded_number("PORTITOR_X", value, 1..=10, 5);
        assert_eq!(read(None), Ok(5));
        assert_eq!(read(Some("")), Ok(5));
        assert_eq!(read(Some("  ")), Ok(5));
        assert_eq!(read(Some(" 1 ")), Ok(1));
        assert_eq!(read(Some("10")), Ok(10));
        for refused in ["0", "11", "-1", "1.5", "2MB"] {
            let error = read(Some(refused)).unwrap_err();
            assert!(
                error.starts_with("PORTITOR_X has an invalid value"),
                "{error}"
            );
            assert!(error.ends_with("from 1 to 10"), "{error}");
        }
    }

    #[test]
    fn items_are_trimmed_and_empty_ones_dropped() {
        assert_eq!(comma_list("a, b,,c "), ["a", "b", "c"]);
        assert!(comma_list("").is_empty());
        assert!(comma_list(" , ").is_empty());
        assert_eq!(
            path_list("/run/a.pem, /run/b.pem"),
            [
                std::path::PathBuf::from("/run/a.pem"),
                std::path::PathBuf::from("/run/b.pem")
            ]
        );
    }
}
