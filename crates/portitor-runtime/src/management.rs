// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The management port, the same in the three binaries (ADR 0023): its
//! settings ([`ManagementConfig`]) and what it serves
//! ([`management_port`]): always the health checks, without a key, and
//! the component's management API, behind its key, when a key file is
//! set.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use tokio::net::TcpListener;

use crate::api_key::ApiKey;
use crate::health::{HealthChecks, management_app};
use crate::serve::bind_management;

/// Where the management port listens, and the API key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagementConfig {
    /// Address to listen on (`PORTITOR_MANAGEMENT_BIND`). The port always
    /// serves the health checks (ADR 0023).
    pub bind: String,
    /// The file that holds the API key
    /// (`PORTITOR_MANAGEMENT_API_KEY_FILE`); without it, the management
    /// API is off and the port serves the health checks only.
    pub api_key_file: Option<PathBuf>,
}

impl ManagementConfig {
    /// Health checks only, on `bind`.
    #[must_use]
    pub fn health_only(bind: &str) -> Self {
        Self {
            bind: bind.to_owned(),
            api_key_file: None,
        }
    }

    /// Reads `PORTITOR_MANAGEMENT_BIND` (default `default_bind`, each
    /// component has its own) and `PORTITOR_MANAGEMENT_API_KEY_FILE` with
    /// `var` (the environment, or a map in tests). Values are trimmed; an
    /// empty value counts as not set, so an empty key file setting leaves
    /// the management API off. Never fails: the `health` subcommand reads
    /// only this.
    pub fn from_vars(default_bind: &str, var: impl Fn(&str) -> Option<OsString>) -> Self {
        let text = |name: &str| {
            var(name)
                .map(|value| value.to_string_lossy().trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        Self {
            bind: text("PORTITOR_MANAGEMENT_BIND").unwrap_or_else(|| default_bind.to_owned()),
            api_key_file: text("PORTITOR_MANAGEMENT_API_KEY_FILE").map(PathBuf::from),
        }
    }

    /// Like [`ManagementConfig::from_vars`], from the environment.
    #[must_use]
    pub fn from_env(default_bind: &str) -> Self {
        Self::from_vars(default_bind, |name| std::env::var_os(name))
    }
}

/// Binds the management port of `config` and returns it with what it
/// serves: the health `checks`, and the management API that `api` makes
/// from the key when a key file is set. Logs when the API is off.
///
/// # Errors
/// The key file cannot be read or holds no valid key (the text starts
/// with `PORTITOR_MANAGEMENT_API_KEY_FILE`), or the address is invalid or
/// taken.
pub async fn management_port(
    config: &ManagementConfig,
    checks: Arc<HealthChecks>,
    api: impl FnOnce(ApiKey) -> Router,
) -> Result<(TcpListener, Router), Box<dyn std::error::Error>> {
    let api = if let Some(key_file) = &config.api_key_file {
        let key =
            ApiKey::read(key_file).map_err(|e| format!("PORTITOR_MANAGEMENT_API_KEY_FILE: {e}"))?;
        Some(api(key))
    } else {
        tracing::info!(
            "management API off (PORTITOR_MANAGEMENT_API_KEY_FILE not set); the management port serves the health checks only"
        );
        None
    };
    let listener = bind_management(&config.bind).await?;
    Ok((listener, management_app(checks, api)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn read(pairs: &'static [(&'static str, &'static str)]) -> ManagementConfig {
        ManagementConfig::from_vars("127.0.0.1:9", |name| {
            pairs
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn values_are_trimmed_and_empty_ones_unset() {
        assert_eq!(read(&[]), ManagementConfig::health_only("127.0.0.1:9"));
        // empty: the management API stays off, the default bind is used
        assert_eq!(
            read(&[
                ("PORTITOR_MANAGEMENT_API_KEY_FILE", "  "),
                ("PORTITOR_MANAGEMENT_BIND", "")
            ]),
            ManagementConfig::health_only("127.0.0.1:9")
        );
        assert_eq!(
            read(&[
                ("PORTITOR_MANAGEMENT_API_KEY_FILE", " /run/key "),
                ("PORTITOR_MANAGEMENT_BIND", " 0.0.0.0:9000 ")
            ]),
            ManagementConfig {
                bind: "0.0.0.0:9000".to_owned(),
                api_key_file: Some(PathBuf::from("/run/key")),
            }
        );
    }

    #[tokio::test]
    async fn a_bad_key_file_names_the_setting() {
        let config = ManagementConfig {
            bind: "127.0.0.1:0".to_owned(),
            api_key_file: Some(PathBuf::from("/nonexistent/portitor-test-key")),
        };
        let checks = Arc::new(HealthChecks::new());
        let error = management_port(&config, checks, |_| Router::new())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with("PORTITOR_MANAGEMENT_API_KEY_FILE: "),
            "{error}"
        );
    }
}
