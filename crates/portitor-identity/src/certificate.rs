// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The X.509 certificate chain of the identity key (ADR 0021, "The key
//! and its certificate"), from `PORTITOR_IDENTITY_CERTIFICATE_CHAIN`:
//!
//! - at start, the first certificate must be for the identity key, or the
//!   identity does not start;
//! - the chain is served at `GET /.well-known/x509CertificateChain.pem`,
//!   and the DID document's JWK points to it with `x5u` (RFC 7517,
//!   section 4.6);
//! - the file is read again every [`RELOAD_INTERVAL`]: a renewed
//!   certificate for the same key replaces the chain without a restart; a
//!   file for another key, or one that cannot be read, is refused and
//!   logged, and the old chain stays.

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use portitor_dcp::Jwk;
use portitor_dcp::x509::first_certificate_key;
use portitor_runtime::api_key::problem;

use crate::Identity;

/// Where the chain is served: the path Gaia-X's own services use (ADR
/// 0021).
pub const CHAIN_PATH: &str = "/.well-known/x509CertificateChain.pem";

/// The media type of the chain: PEM certificates (RFC 7468, section 5.3,
/// `application/pem-certificate-chain`, RFC 8555 section 9.1).
pub const CHAIN_MEDIA_TYPE: &str = "application/pem-certificate-chain";

/// How often the file is read again.
pub const RELOAD_INTERVAL: Duration = Duration::from_secs(60);

/// The chain in use, and where it came from.
#[derive(Debug)]
pub struct CertificateChain {
    file: PathBuf,
    key: Jwk,
    pem: RwLock<String>,
}

/// What [`CertificateChain::reload`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reload {
    /// The file holds the chain in use.
    Unchanged,
    /// The file holds a new chain for the same key, now in use.
    Replaced,
    /// The file cannot be used; the old chain stays. The text says why.
    Refused(String),
}

impl CertificateChain {
    /// Reads `file` and checks that its first certificate is for `key`
    /// (the identity key's public JWK).
    ///
    /// # Errors
    /// The file cannot be read, is not a PEM chain, or its first
    /// certificate is for another key.
    pub fn load(file: &Path, key: &Jwk) -> Result<Self, String> {
        let pem = read_checked(file, key)?;
        Ok(Self {
            file: file.to_owned(),
            key: key.clone(),
            pem: RwLock::new(pem),
        })
    }

    /// The chain in use, PEM.
    #[must_use]
    pub fn pem(&self) -> String {
        // a panic while holding the lock leaves a whole string behind
        self.pem
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Reads the file again and uses it if it changed and its first
    /// certificate is still for the identity key.
    pub fn reload(&self) -> Reload {
        let pem = match read_checked(&self.file, &self.key) {
            Ok(pem) => pem,
            Err(reason) => return Reload::Refused(reason),
        };
        let mut current = self.pem.write().unwrap_or_else(PoisonError::into_inner);
        if *current == pem {
            return Reload::Unchanged;
        }
        *current = pem;
        Reload::Replaced
    }
}

/// The file's text, if its first certificate is for `key`.
fn read_checked(file: &Path, key: &Jwk) -> Result<String, String> {
    let pem = std::fs::read_to_string(file)
        .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let certified = first_certificate_key(&pem).map_err(|e| format!("{}: {e}", file.display()))?;
    if certified != *key {
        return Err(format!(
            "{}: the first certificate is for another key than the identity key",
            file.display()
        ));
    }
    Ok(pem)
}

/// Reads the file again every [`RELOAD_INTERVAL`], until the process
/// stops, and logs what changed.
pub async fn watch(chain: Arc<CertificateChain>) {
    loop {
        tokio::time::sleep(RELOAD_INTERVAL).await;
        match chain.reload() {
            Reload::Unchanged => {}
            Reload::Replaced => {
                tracing::info!(file = %chain.file.display(), "certificate chain reloaded");
            }
            Reload::Refused(reason) => {
                tracing::error!(%reason, "new certificate chain refused; the old one stays");
            }
        }
    }
}

/// `GET /.well-known/x509CertificateChain.pem`.
pub(crate) async fn serve(State(identity): State<Arc<Identity>>) -> Response {
    match &identity.certificate {
        Some(chain) => ([(header::CONTENT_TYPE, CHAIN_MEDIA_TYPE)], chain.pem()).into_response(),
        None => problem(
            StatusCode::NOT_FOUND,
            "no certificate chain (PORTITOR_IDENTITY_CERTIFICATE_CHAIN)",
        ),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use portitor_dcp::SigningKey;

    const KEY: &str = include_str!("../tests/fixtures/test-only-identity-key.pem");
    const CHAIN: &str = include_str!("../tests/fixtures/test-only-identity-chain.pem");
    const OTHER: &str = include_str!("../tests/fixtures/test-only-other-cert.pem");

    #[test]
    fn a_new_chain_must_be_for_the_same_key() {
        let dir = std::env::temp_dir().join(format!(
            "portitor-identity-chain-{}",
            portitor_runtime::system::new_uuid().trim_start_matches("urn:uuid:")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("chain.pem");
        let key = SigningKey::from_pem(KEY).unwrap().public_jwk();

        std::fs::write(&file, OTHER).unwrap();
        let refused = CertificateChain::load(&file, &key).unwrap_err();
        assert!(refused.contains("another key"), "{refused}");

        std::fs::write(&file, CHAIN).unwrap();
        let chain = CertificateChain::load(&file, &key).unwrap();
        assert_eq!(chain.pem(), CHAIN);
        assert_eq!(chain.reload(), Reload::Unchanged);

        // a renewed certificate (here: the same one, with a comment) is used
        let renewed = format!("renewed\n{CHAIN}");
        std::fs::write(&file, &renewed).unwrap();
        assert_eq!(chain.reload(), Reload::Replaced);
        assert_eq!(chain.pem(), renewed);

        // a chain for another key, or a broken file, is refused
        std::fs::write(&file, OTHER).unwrap();
        assert!(matches!(chain.reload(), Reload::Refused(_)));
        std::fs::write(&file, "garbage").unwrap();
        assert!(matches!(chain.reload(), Reload::Refused(_)));
        std::fs::remove_file(&file).unwrap();
        assert!(matches!(chain.reload(), Reload::Refused(_)));
        assert_eq!(chain.pem(), renewed, "the old chain stays");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
