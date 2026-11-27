// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The issuer's settings, read from environment variables
//! (`docs/DEPLOYING.md`, "Step 1: issuer", "Issuer settings"). The names follow
//! the identity component's.

use std::ffi::OsString;
use std::path::PathBuf;

use portitor_runtime::database;
pub use portitor_runtime::database::DatabaseSetting;

pub use portitor_runtime::settings::base_path;
use portitor_runtime::settings::{self, ALLOW_HTTP_CALLBACKS, RemovedSetting, refuse_removed};

/// Where the public server listens unless `PORTITOR_BIND` says otherwise.
pub const DEFAULT_BIND: &str = "0.0.0.0:8070";

/// Where the management API listens unless `PORTITOR_MANAGEMENT_BIND`
/// says otherwise: on this machine only.
pub const DEFAULT_MANAGEMENT_BIND: &str = "127.0.0.1:8071";

/// The `SQLite` file unless `PORTITOR_DB_FILE` (or
/// `PORTITOR_DATABASE_URL`) says otherwise.
pub const DEFAULT_DB_FILE: &str = "/data/portitor-issuer.db";

/// How many credentials of one definition a holder may have that are
/// neither revoked nor expired, unless
/// `PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER` says otherwise. It keeps one
/// registered participant from filling a status list (ADR 0018, amended
/// during the 0.6 review).
pub const DEFAULT_MAX_ACTIVE_PER_HOLDER: u64 = 10;

/// The settings the issuer no longer has. When one of them is set, the
/// issuer refuses to start and names what to set instead.
pub const REMOVED_SETTINGS: &[RemovedSetting] = &[
    ALLOW_HTTP_CALLBACKS,
    // the file holds credential definitions, not credentials
    RemovedSetting {
        name: "PORTITOR_ISSUER_CREDENTIALS_FILE",
        advice: "was renamed in 0.13; set PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE instead",
    },
];

/// Everything the issuer needs to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The issuer's `did:web` DID (`PORTITOR_PARTICIPANT_ID`).
    pub participant_id: String,
    /// The P-256 private key, PEM (`PORTITOR_ISSUER_KEY_FILE`).
    pub key_file: PathBuf,
    /// Public keys that signed before a key change, one PEM file each,
    /// listed in the DID document but never used to sign
    /// (`PORTITOR_ISSUER_RETIRED_KEY_FILES`, comma-separated; TD-05).
    pub retired_key_files: Vec<PathBuf>,
    /// The credential definitions, JSON (`PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE`).
    pub credential_definitions_file: PathBuf,
    /// The issuer service's public base URL, written into the DID
    /// document as `IssuerService` (`PORTITOR_ISSUER_SERVICE_URL`). The
    /// status lists are served under it too, so it must not change once
    /// credentials are issued.
    pub issuer_service_url: String,
    /// Where the public server listens (`PORTITOR_BIND`).
    pub bind: String,
    /// The database: PostgreSQL (`PORTITOR_DATABASE_URL` and
    /// `PORTITOR_DATABASE_PASSWORD_FILE`), or the `SQLite` file
    /// (`PORTITOR_DB_FILE`, [`DEFAULT_DB_FILE`] when not set).
    pub database: DatabaseSetting,
    /// The management port: always the health checks, and the management
    /// API when `PORTITOR_MANAGEMENT_API_KEY_FILE` is set.
    pub management: ManagementConfig,
    /// Plain-HTTP DIDs and URLs allowed (`PORTITOR_ALLOW_HTTP`,
    /// tests only).
    pub allow_http: bool,
    /// The most credentials of one definition a holder may have that are
    /// neither revoked nor expired; a request beyond it ends `REJECTED`
    /// (`PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER`, at least 1).
    pub max_active_per_holder: u64,
    /// The public bundle key, listed in the DID document as
    /// `<did>#bundle-key` (`PORTITOR_ISSUER_BUNDLE_KEY_FILE`, optional; a
    /// JWK or a PKCS#8 PEM, see [`crate::bundles::public_key_from_text`]).
    pub bundle_key_file: Option<PathBuf>,
    /// The folder of built bundles, served read-only at
    /// `/bundles/{version}/{file}` (`PORTITOR_ISSUER_BUNDLE_DIR`, optional).
    pub bundle_dir: Option<PathBuf>,
}

/// Where the management port listens, and the API key: the settings
/// shared by the three components, read with
/// `ManagementConfig::from_env(DEFAULT_MANAGEMENT_BIND)`.
pub use portitor_runtime::management::ManagementConfig;

/// A setting is missing or wrong.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{name}: {problem}")]
pub struct ConfigError {
    /// The environment variable.
    pub name: &'static str,
    /// What is wrong with it.
    pub problem: String,
}

impl Config {
    /// Reads the settings from the environment.
    ///
    /// # Errors
    /// A required setting is missing or a setting is invalid.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(|name| std::env::var_os(name))
    }

    /// Reads the settings with `var` (the environment, or a map in tests).
    ///
    /// # Errors
    /// A required setting is missing or a setting is invalid.
    pub fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Result<Self, ConfigError> {
        // trimmed, and `None` for an empty value: empty means "not set"
        // (the rule of every setting, `portitor_runtime::settings`)
        let text = |name: &str| settings::text(&var, name);
        let required = |name: &'static str| text(name).ok_or_else(|| invalid(name, "required"));
        refuse_removed(&var, REMOVED_SETTINGS).map_err(removed)?;
        let participant_id = required("PORTITOR_PARTICIPANT_ID")?;
        portitor_dcp::did_web_url(&participant_id)
            .map_err(|e| invalid("PORTITOR_PARTICIPANT_ID", &e.to_string()))?;
        let issuer_service_url = required("PORTITOR_ISSUER_SERVICE_URL")?;
        if base_path(&issuer_service_url).is_none() {
            return Err(invalid("PORTITOR_ISSUER_SERVICE_URL", "not an http(s) URL"));
        }
        let max_active_per_holder = match text("PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER") {
            None => DEFAULT_MAX_ACTIVE_PER_HOLDER,
            Some(value) => match value.parse::<u64>() {
                Ok(limit) if limit >= 1 => limit,
                _ => {
                    return Err(invalid(
                        "PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER",
                        "not a whole number of at least 1",
                    ));
                }
            },
        };
        let management = ManagementConfig::from_vars(DEFAULT_MANAGEMENT_BIND, &var);
        // a list that is set must name something (`settings::list`)
        let retired_key_files = settings::list(&var, "PORTITOR_ISSUER_RETIRED_KEY_FILES")
            .map_err(|e| invalid("PORTITOR_ISSUER_RETIRED_KEY_FILES", &e))?
            .unwrap_or_default();
        let allow_http = settings::switch(&var, "PORTITOR_ALLOW_HTTP")
            .map_err(|e| invalid("PORTITOR_ALLOW_HTTP", &e))?;
        let database = database::setting(&var, DEFAULT_DB_FILE)
            .map_err(|e| invalid("PORTITOR_DATABASE_URL", &e))?;
        Ok(Self {
            participant_id,
            key_file: PathBuf::from(required("PORTITOR_ISSUER_KEY_FILE")?),
            retired_key_files: retired_key_files.into_iter().map(PathBuf::from).collect(),
            credential_definitions_file: PathBuf::from(required(
                "PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE",
            )?),
            issuer_service_url: issuer_service_url.trim_end_matches('/').to_owned(),
            bind: text("PORTITOR_BIND").unwrap_or_else(|| DEFAULT_BIND.to_owned()),
            database,
            management,
            allow_http,
            max_active_per_holder,
            bundle_key_file: text("PORTITOR_ISSUER_BUNDLE_KEY_FILE").map(PathBuf::from),
            bundle_dir: text("PORTITOR_ISSUER_BUNDLE_DIR").map(PathBuf::from),
        })
    }
}

/// The error for a removed setting that is still set: its old name, and
/// what to set instead.
fn removed(setting: RemovedSetting) -> ConfigError {
    invalid(setting.name, setting.advice)
}

fn invalid(name: &'static str, problem: &str) -> ConfigError {
    ConfigError {
        name,
        problem: problem.to_owned(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn read(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        Config::from_vars(portitor_runtime::testing::vars(pairs))
    }

    /// Old names (refused) and the new name each error must give.
    const RENAMED: &[(&str, &str)] = &[
        ("PORTITOR_ALLOW_HTTP_CALLBACKS", "PORTITOR_ALLOW_HTTP"),
        (
            "PORTITOR_ISSUER_CREDENTIALS_FILE",
            "PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE",
        ),
    ];

    const MINIMAL: &[(&str, &str)] = &[
        ("PORTITOR_PARTICIPANT_ID", "did:web:issuer.example"),
        ("PORTITOR_ISSUER_KEY_FILE", "/run/secrets/issuer.pem"),
        (
            "PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE",
            "/config/credentials.json",
        ),
        (
            "PORTITOR_ISSUER_SERVICE_URL",
            "https://issuer.example/issuer/",
        ),
    ];

    #[test]
    fn a_renamed_setting_is_refused_with_its_new_name() {
        for (old, new) in RENAMED {
            let mut vars = MINIMAL.to_vec();
            vars.push((old, "anything"));
            let error = read(&vars).unwrap_err();
            assert_eq!(error.name, *old);
            assert!(error.to_string().contains(new), "{error}");
        }
    }

    /// An item of a list setting has a length limit
    /// (`settings::LIST_ITEM_MAX_BYTES`): a longer one stops the start,
    /// and the error names the setting and nothing of the item.
    #[test]
    fn a_list_item_that_is_too_long_is_invalid_with_its_name() {
        let too_long = format!("/run/keys/{}", "k".repeat(settings::LIST_ITEM_MAX_BYTES));
        let value = format!("/run/keys/2025.pub.pem,{too_long}");
        let mut vars = MINIMAL.to_vec();
        vars.push(("PORTITOR_ISSUER_RETIRED_KEY_FILES", value.as_str()));
        let error = read(&vars).unwrap_err();
        assert_eq!(error.name, "PORTITOR_ISSUER_RETIRED_KEY_FILES");
        assert!(
            error
                .problem
                .starts_with("has an item that is too long: item 2 is 4106 bytes"),
            "{error}"
        );
        assert!(!error.to_string().contains("kkk"), "{error}");
    }

    /// A list that is set names something, and a switch is `true` or
    /// `false`: anything else stops the start.
    #[test]
    fn a_list_of_commas_or_a_switch_that_is_not_true_or_false_is_refused() {
        for (name, value) in [
            ("PORTITOR_ISSUER_RETIRED_KEY_FILES", ",,"),
            ("PORTITOR_ALLOW_HTTP", "1"),
            ("PORTITOR_ALLOW_HTTP", "yes"),
        ] {
            let mut vars = MINIMAL.to_vec();
            vars.push((name, value));
            let error = read(&vars).unwrap_err();
            assert_eq!(error.name, name, "{error}");
        }
        let mut vars = MINIMAL.to_vec();
        vars.push(("PORTITOR_ALLOW_HTTP", "FALSE"));
        assert!(!read(&vars).unwrap().allow_http);
    }

    /// The rule of every setting: spaces around a value are dropped, and
    /// an empty value means "not set".
    #[test]
    fn an_empty_value_is_not_set_and_values_are_trimmed() {
        // an optional setting that is empty takes its default
        let mut vars = MINIMAL.to_vec();
        for optional in [
            "PORTITOR_BIND",
            "PORTITOR_DB_FILE",
            "PORTITOR_DATABASE_URL",
            "PORTITOR_DATABASE_PASSWORD_FILE",
            "PORTITOR_ISSUER_RETIRED_KEY_FILES",
            "PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER",
            "PORTITOR_ISSUER_BUNDLE_KEY_FILE",
            "PORTITOR_ISSUER_BUNDLE_DIR",
            "PORTITOR_ALLOW_HTTP",
            "PORTITOR_MANAGEMENT_BIND",
            "PORTITOR_MANAGEMENT_API_KEY_FILE",
        ] {
            vars.push((optional, " "));
        }
        assert_eq!(read(&vars).unwrap(), read(MINIMAL).unwrap());
        // a required setting that is empty is missing
        for required in MINIMAL.iter().map(|(name, _)| *name) {
            let mut vars = MINIMAL.to_vec();
            vars.push((required, ""));
            assert_eq!(read(&vars).unwrap_err(), invalid(required, "required"));
        }
        // spaces around a value are dropped
        let mut vars = MINIMAL.to_vec();
        vars.push(("PORTITOR_BIND", " 127.0.0.1:9 "));
        vars.push(("PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER", " 3 "));
        let config = read(&vars).unwrap();
        assert_eq!(config.bind, "127.0.0.1:9");
        assert_eq!(config.max_active_per_holder, 3);
    }

    #[test]
    fn defaults() {
        let config = read(MINIMAL).unwrap();
        assert_eq!(config.bind, "0.0.0.0:8070");
        assert_eq!(
            config.database,
            DatabaseSetting::SqliteFile(PathBuf::from("/data/portitor-issuer.db"))
        );
        assert_eq!(config.issuer_service_url, "https://issuer.example/issuer");
        assert_eq!(
            config.management,
            ManagementConfig::health_only(DEFAULT_MANAGEMENT_BIND)
        );
        assert_eq!(config.management.bind, "127.0.0.1:8071");
        assert!(!config.allow_http);
        assert_eq!(config.max_active_per_holder, 10);
        assert_eq!(config.bundle_key_file, None);
        assert_eq!(config.bundle_dir, None);
        assert!(config.retired_key_files.is_empty());
    }

    #[test]
    fn retired_key_files_are_a_comma_separated_list() {
        let mut vars = MINIMAL.to_vec();
        vars.push((
            "PORTITOR_ISSUER_RETIRED_KEY_FILES",
            "/run/keys/2025.pub.pem,/run/keys/2026.pub.pem",
        ));
        assert_eq!(
            read(&vars).unwrap().retired_key_files,
            [
                PathBuf::from("/run/keys/2025.pub.pem"),
                PathBuf::from("/run/keys/2026.pub.pem")
            ]
        );
    }

    #[test]
    fn bundle_settings() {
        let mut vars = MINIMAL.to_vec();
        vars.push(("PORTITOR_ISSUER_BUNDLE_KEY_FILE", "/run/secrets/bundle.jwk"));
        vars.push(("PORTITOR_ISSUER_BUNDLE_DIR", "/bundles"));
        let config = read(&vars).unwrap();
        assert_eq!(
            config.bundle_key_file,
            Some(PathBuf::from("/run/secrets/bundle.jwk"))
        );
        assert_eq!(config.bundle_dir, Some(PathBuf::from("/bundles")));
    }

    #[test]
    fn management_api_needs_a_key_file() {
        let mut vars = MINIMAL.to_vec();
        vars.push(("PORTITOR_MANAGEMENT_API_KEY_FILE", "/run/secrets/api"));
        vars.push(("PORTITOR_ALLOW_HTTP", "TRUE"));
        let config = read(&vars).unwrap();
        assert_eq!(
            config.management,
            ManagementConfig {
                bind: "127.0.0.1:8071".to_owned(),
                api_key_file: Some(PathBuf::from("/run/secrets/api"))
            }
        );
        assert!(config.allow_http);
    }

    #[test]
    fn required_and_invalid_settings() {
        for missing in MINIMAL.iter().map(|(name, _)| *name) {
            let vars: Vec<_> = MINIMAL
                .iter()
                .copied()
                .filter(|(name, _)| *name != missing)
                .collect();
            assert_eq!(read(&vars).unwrap_err().name, missing);
        }
        let mut vars = MINIMAL.to_vec();
        vars.push(("PORTITOR_PARTICIPANT_ID", "did:key:z6Mk"));
        assert!(read(&vars).is_err());
        let mut vars = MINIMAL.to_vec();
        vars.push(("PORTITOR_ISSUER_SERVICE_URL", "ftp://issuer.example"));
        assert_eq!(read(&vars).unwrap_err().name, "PORTITOR_ISSUER_SERVICE_URL");
        for wrong in ["0", "-1", "ten"] {
            let mut vars = MINIMAL.to_vec();
            vars.push(("PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER", wrong));
            assert_eq!(
                read(&vars).unwrap_err().name,
                "PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER"
            );
        }
        let mut vars = MINIMAL.to_vec();
        vars.push(("PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER", "50"));
        assert_eq!(read(&vars).unwrap().max_active_per_holder, 50);
    }
}
