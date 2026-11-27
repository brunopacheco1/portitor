// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The tools of the tests named `settings` in the three binaries'
//! crates: they check, for every setting of a binary, the one rule of
//! `crate::settings` (from 0.13): spaces around a value are dropped, and
//! an empty value means "not set".
//!
//! A binary's test gives a table with one [`Case`] per setting and a
//! function that reads all settings from a lookup and returns what it
//! read as text. The checks here change one setting at a time and
//! compare the texts.
//!
//! The functions return an error text instead of panicking; the tests
//! print it.

use super::{Lookup, vars};
use crate::settings::RemovedSetting;

/// One setting in the table of a binary's test.
#[derive(Debug, Clone)]
pub struct Case {
    /// The environment variable.
    pub name: &'static str,
    /// A value the binary accepts.
    pub value: String,
    /// Another value the binary accepts, which must change what is read.
    pub other: String,
    /// Whether the binary refuses to start without the setting, when
    /// every other setting of the table is set.
    pub required: bool,
}

impl Case {
    /// A setting the binary starts without.
    #[must_use]
    pub fn optional(name: &'static str, value: &str, other: &str) -> Self {
        Self {
            name,
            value: value.to_owned(),
            other: other.to_owned(),
            required: false,
        }
    }

    /// A setting the binary does not start without.
    #[must_use]
    pub fn required(name: &'static str, value: &str, other: &str) -> Self {
        Self {
            required: true,
            ..Self::optional(name, value, other)
        }
    }
}

/// What a binary read from a lookup, as text (the `Debug` text of its
/// settings), or why it refuses to start.
pub type Read = Result<String, String>;

/// The lookup with every setting of `cases` at its `value`, apart from
/// the setting `changed`: that one has the value `to`, or is not set
/// when `to` is `None`.
fn lookup_with(cases: &[Case], changed: &str, to: Option<&str>) -> Lookup {
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    for case in cases {
        if case.name != changed {
            pairs.push((case.name, case.value.as_str()));
        } else if let Some(value) = to {
            pairs.push((case.name, value));
        }
    }
    vars(&pairs)
}

/// Checks the rule for every setting of `cases`, with `read`. For each
/// setting, while every other one has its `value`:
///
/// 1. the setting is read at all: its `other` value changes what is read;
/// 2. spaces, a tab and a line end around the value change nothing;
/// 3. an empty value, and one of spaces only, is the same as no value;
/// 4. without the setting, the binary starts if it is optional, and if
///    it is required refuses with a text that names the setting.
///
/// # Errors
/// Every check that failed, one per line.
pub fn check_rule(cases: &[Case], read: impl Fn(&Lookup) -> Read) -> Result<(), String> {
    let mut failures: Vec<String> = Vec::new();
    let all_set = match read(&lookup_with(cases, "", None)) {
        Ok(text) => text,
        Err(error) => return Err(format!("the table's values are refused: {error}")),
    };
    for case in cases {
        let name = case.name;
        let read_with = |value: Option<&str>| read(&lookup_with(cases, name, value));

        if read_with(Some(&case.other)).as_ref() == Ok(&all_set) {
            failures.push(format!(
                "{name}: the value {:?} in place of {:?} changes nothing; is it read?",
                case.other, case.value
            ));
        }
        let padded = format!(" \t{} \n", case.value);
        let read_padded = read_with(Some(&padded));
        if read_padded.as_ref() != Ok(&all_set) {
            failures.push(format!(
                "{name}: spaces around the value are not dropped: {read_padded:?}"
            ));
        }
        let not_set = read_with(None);
        for empty in ["", " ", " \t\n "] {
            let read_empty = read_with(Some(empty));
            if read_empty != not_set {
                failures.push(format!(
                    "{name}: the empty value {empty:?} is not the same as no value: \
                     {read_empty:?}, without it {not_set:?}"
                ));
            }
        }
        match (&not_set, case.required) {
            (Ok(_), false) => {}
            (Err(error), true) if error.contains(name) => {}
            (Err(error), true) => failures.push(format!(
                "{name}: the refusal without it does not name it: {error}"
            )),
            (Ok(_), true) => failures.push(format!(
                "{name}: listed as required, but the binary starts without it"
            )),
            (Err(error), false) => failures.push(format!(
                "{name}: listed as optional, but the binary refuses to start without it: {error}"
            )),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// Checks the names a release took away (`removed`), with `read`, while
/// every setting of `cases` is set:
///
/// 1. an old name that is set stops the start, also when it is set to
///    `false` and also when its new name is set too (`cases` holds the
///    new names); the refusal names the old name and says what to do;
/// 2. an old name set to an empty value is the same as not set (the rule
///    of every setting), so the binary starts.
///
/// # Errors
/// Every check that failed, one per line.
pub fn check_removed(
    removed: &[RemovedSetting],
    cases: &[Case],
    read: impl Fn(&Lookup) -> Read,
) -> Result<(), String> {
    let mut failures: Vec<String> = Vec::new();
    let read_with_old_name = |name: &str, value: &str| {
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        for case in cases {
            pairs.push((case.name, case.value.as_str()));
        }
        pairs.push((name, value));
        read(&vars(&pairs))
    };
    let all_set = read(&lookup_with(cases, "", None));
    for setting in removed {
        for value in ["true", "false", " x ", "/a/file"] {
            match read_with_old_name(setting.name, value) {
                Err(error) if error.contains(setting.name) && error.contains(setting.advice) => {}
                other => failures.push(format!(
                    "{}={value:?} is not refused with its advice: {other:?}",
                    setting.name
                )),
            }
        }
        for empty in ["", "  "] {
            let read_empty = read_with_old_name(setting.name, empty);
            if read_empty != all_set {
                failures.push(format!(
                    "{}={empty:?} is not the same as not set: {read_empty:?}",
                    setting.name
                ));
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// What a switch (a setting that is on or off) makes of each way an
/// operator may write "on" or "off": one line per value, `"<value>" on`,
/// `"<value>" off` or `"<value>" refused`. `read_switch` reads the
/// switch from a lookup that holds every setting of `cases` and the
/// switch `name` with the value.
///
/// The tests compare the lines with [`SWITCH_TRUTH_TABLE`], the same for
/// every switch of the three binaries.
pub fn switch_truth_table(
    name: &str,
    cases: &[Case],
    read_switch: impl Fn(&Lookup) -> Result<bool, String>,
) -> Vec<String> {
    let mut lines = Vec::new();
    for value in SWITCH_VALUES {
        let outcome = match read_switch(&lookup_with_extra(cases, name, value)) {
            Ok(true) => "on",
            Ok(false) => "off",
            Err(_) => "refused",
        };
        lines.push(format!("{value:?} {outcome}"));
    }
    lines
}

/// Like [`lookup_with`], for a setting that may not be in `cases`.
fn lookup_with_extra(cases: &[Case], name: &str, value: &str) -> Lookup {
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    for case in cases {
        pairs.push((case.name, case.value.as_str()));
    }
    // the last value of a name wins
    pairs.push((name, value));
    vars(&pairs)
}

/// The values [`switch_truth_table`] tries.
const SWITCH_VALUES: [&str; 20] = [
    "true",
    "TRUE",
    "True",
    "tRuE",
    " true ",
    "true\n",
    "1",
    "yes",
    "on",
    "y",
    "t",
    "enabled",
    "false",
    "0",
    "no",
    "off",
    "",
    "truee",
    "\"true\"",
    "true false",
];

/// What every switch of the three binaries makes of [`SWITCH_VALUES`]:
/// on for `true` in any letter case, with or without spaces around it;
/// off for `false` and for an empty value; everything else is refused,
/// so the binary does not start.
pub const SWITCH_TRUTH_TABLE: [&str; 20] = [
    "\"true\" on",
    "\"TRUE\" on",
    "\"True\" on",
    "\"tRuE\" on",
    "\" true \" on",
    "\"true\\n\" on",
    "\"1\" refused",
    "\"yes\" refused",
    "\"on\" refused",
    "\"y\" refused",
    "\"t\" refused",
    "\"enabled\" refused",
    "\"false\" off",
    "\"0\" refused",
    "\"no\" refused",
    "\"off\" refused",
    "\"\" off",
    "\"truee\" refused",
    "\"\\\"true\\\"\" refused",
    "\"true false\" refused",
];

/// Checks the real binary at the path `binary` (in a test:
/// `env!("CARGO_BIN_EXE_<name>")`): it does not start without settings,
/// and not with a removed name. Each run has only the settings named
/// here, so it always ends at once.
///
/// 1. No setting at all: the binary stops and names `first_required`.
/// 2. Only an old name of `removed` set: the binary stops and says what
///    to set instead. This is checked before anything else, so the
///    operator reads about the old name first, not about a setting that
///    is missing.
///
/// # Errors
/// The binary cannot be run, or a check failed; one per line.
pub fn check_binary_refuses_to_start(
    binary: &str,
    first_required: &str,
    removed: &[RemovedSetting],
) -> Result<(), String> {
    let mut failures: Vec<String> = Vec::new();
    // what the binary wrote, when it stopped with a failure
    let stopped_with = |settings: &[(&str, &str)]| -> Result<String, String> {
        let output = std::process::Command::new(binary)
            // nothing of the test machine's own settings
            .env_clear()
            .envs(settings.iter().copied())
            .output()
            .map_err(|error| format!("cannot run {binary}: {error}"))?;
        let mut written = String::from_utf8_lossy(&output.stdout).into_owned();
        written.push_str(&String::from_utf8_lossy(&output.stderr));
        if output.status.success() {
            return Err(format!("{binary} ended without a failure: {written}"));
        }
        Ok(written)
    };
    let written = stopped_with(&[])?;
    if !written.contains(first_required) {
        failures.push(format!(
            "without settings, {first_required} is not named: {written}"
        ));
    }
    for setting in removed {
        let written = stopped_with(&[(setting.name, "true")])?;
        if !written.contains(setting.name) || !written.contains(setting.advice) {
            failures.push(format!(
                "with {} set, its advice is not given: {written}",
                setting.name
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// Checks the real binary at the path `binary` reads `name` at start: with
/// only `name` set, to the wrong `value`, it stops and names `name`. So
/// the setting is read before any required one is missing.
///
/// # Errors
/// The binary cannot be run, it started, or it did not name `name`.
pub fn check_binary_refuses_value(binary: &str, name: &str, value: &str) -> Result<(), String> {
    let output = std::process::Command::new(binary)
        // nothing of the test machine's own settings
        .env_clear()
        .env(name, value)
        .output()
        .map_err(|error| format!("cannot run {binary}: {error}"))?;
    let mut written = String::from_utf8_lossy(&output.stdout).into_owned();
    written.push_str(&String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        return Err(format!("{binary} ended without a failure: {written}"));
    }
    if !written.contains(name) {
        return Err(format!(
            "with {name}={value}, {name} is not named: {written}"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::settings;

    fn table() -> Vec<Case> {
        vec![
            Case::required("T_NAME", "a", "b"),
            Case::optional("T_PORT", "1", "2"),
        ]
    }

    /// A reader that keeps the rule.
    fn good(var: &Lookup) -> Read {
        let name = settings::text(var, "T_NAME").ok_or("T_NAME: required")?;
        let port = settings::text(var, "T_PORT").unwrap_or_else(|| "80".to_owned());
        settings::refuse_removed(var, &[OLD]).map_err(|removed| removed.to_string())?;
        Ok(format!("{name} {port}"))
    }

    const OLD: RemovedSetting = RemovedSetting {
        name: "T_OLD",
        advice: "was renamed; set T_NAME instead",
    };

    /// The value of `name` as the environment gives it, not trimmed.
    fn raw(var: &Lookup, name: &str) -> Option<String> {
        var(name).map(|value| value.to_string_lossy().into_owned())
    }

    #[test]
    fn a_reader_that_keeps_the_rule_passes() {
        assert_eq!(check_rule(&table(), good), Ok(()));
        assert_eq!(check_removed(&[OLD], &table(), good), Ok(()));
    }

    /// The checks are tried on readers with one fault each: every fault
    /// is found, and the text names the setting.
    #[test]
    fn each_way_to_break_the_rule_is_found() {
        let does_not_trim = |var: &Lookup| -> Read {
            let name = raw(var, "T_NAME").filter(|value| !value.trim().is_empty());
            Ok(format!(
                "{} {:?}",
                name.ok_or("T_NAME: required")?,
                settings::text(var, "T_PORT")
            ))
        };
        let error = check_rule(&table(), does_not_trim).unwrap_err();
        assert!(error.contains("T_NAME: spaces around"), "{error}");

        let takes_empty_as_set = |var: &Lookup| -> Read {
            let port = raw(var, "T_PORT").map(|value| value.trim().to_owned());
            Ok(format!("{} {port:?}", good(var)?))
        };
        let error = check_rule(&table(), takes_empty_as_set).unwrap_err();
        assert!(error.contains("T_PORT: the empty value"), "{error}");

        let does_not_read_the_port = |var: &Lookup| -> Read {
            settings::text(var, "T_NAME").ok_or_else(|| "T_NAME: required".to_owned())
        };
        let error = check_rule(&table(), does_not_read_the_port).unwrap_err();
        assert!(error.contains("T_PORT: the value"), "{error}");

        let does_not_name_what_is_missing = |var: &Lookup| -> Read {
            good(var).map_err(|_| "a required setting is missing".to_owned())
        };
        let error = check_rule(&table(), does_not_name_what_is_missing).unwrap_err();
        assert!(error.contains("T_NAME: the refusal"), "{error}");

        let needs_nothing = |var: &Lookup| -> Read {
            Ok(format!(
                "{:?} {:?}",
                settings::text(var, "T_NAME"),
                settings::text(var, "T_PORT")
            ))
        };
        let error = check_rule(&table(), needs_nothing).unwrap_err();
        assert!(error.contains("T_NAME: listed as required"), "{error}");
        let error = check_removed(&[OLD], &table(), needs_nothing).unwrap_err();
        assert!(error.contains("T_OLD=\"true\" is not refused"), "{error}");

        let refuses_an_empty_old_name = |var: &Lookup| -> Read {
            if var("T_OLD").is_some() {
                return Err(OLD.to_string());
            }
            good(var)
        };
        let error = check_removed(&[OLD], &table(), refuses_an_empty_old_name).unwrap_err();
        assert!(error.contains("T_OLD=\"\" is not the same"), "{error}");

        let mut wrong_table = table();
        wrong_table[0].value = String::new();
        let error = check_rule(&wrong_table, good).unwrap_err();
        assert!(error.contains("the table's values are refused"), "{error}");
    }

    #[test]
    fn the_shared_switch_reader_has_the_truth_table() {
        let read = |var: &Lookup| settings::switch(var, "T_SWITCH");
        assert_eq!(
            switch_truth_table("T_SWITCH", &table(), read),
            SWITCH_TRUTH_TABLE
        );
        // a reader that takes what it does not know as off is seen
        let lenient = |var: &Lookup| {
            let value = settings::text(var, "T_SWITCH");
            Ok(value.is_some_and(|value| value.eq_ignore_ascii_case("true")))
        };
        let lines = switch_truth_table("T_SWITCH", &table(), lenient);
        assert!(lines.contains(&"\"yes\" off".to_owned()), "{lines:?}");
    }
}
