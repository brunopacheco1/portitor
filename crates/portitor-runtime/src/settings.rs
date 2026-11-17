// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Helpers for reading settings from environment variables, shared by
//! the connector, the identity component and the issuer.

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
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    if host.is_empty() || path.contains(['?', '#']) {
        return None;
    }
    Some(format!("/{path}").trim_end_matches('/').to_owned())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

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
