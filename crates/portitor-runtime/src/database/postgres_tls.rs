// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! TLS for PostgreSQL connections, on the rustls (ring provider) we use
//! for HTTPS. The server's certificate and host name are always checked
//! (libpq's `verify-full`), against the CAs that `sslrootcert` names, as
//! libpq does: only those of the PEM file it names, or the system's CA
//! certificates for `sslrootcert=system`. Without `sslrootcert`, the
//! system's CA certificates (libpq reads `~/.postgresql/root.crt` then,
//! which a container does not have).
//!
//! Written here rather than taken from `tokio-postgres-rustls`, which
//! brings a bundled root store and more crates (ADR 0029).
//!
//! No channel binding (SCRAM-SHA-256-PLUS): `tokio-postgres` then uses
//! SCRAM-SHA-256, which PostgreSQL accepts unless the server demands
//! channel binding.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::tls::{ChannelBinding, MakeTlsConnect, TlsConnect, TlsStream};

/// The TLS settings for PostgreSQL: the CAs of `ca_file` (a PEM file)
/// and no others, or the system's CA certificates when `ca_file` is
/// missing or `system`.
///
/// # Errors
/// `ca_file` cannot be read or holds no certificate, or there is no CA
/// certificate at all.
pub(super) fn client_config(ca_file: Option<&str>) -> Result<Arc<rustls::ClientConfig>, String> {
    let roots = match ca_file {
        None | Some("system") => system_roots(),
        Some(path) => roots_of_file(path)?,
    };
    if roots.is_empty() {
        return Err("no CA certificate to check the database server with; give \
             sslrootcert in PORTITOR_DATABASE_URL"
            .to_owned());
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS: {e}"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// The system's CA certificates. The system's store may hold a few
/// certificates rustls cannot read; the rest are enough.
fn system_roots() -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    let (_, _) = roots.add_parsable_certificates(native.certs);
    roots
}

/// The CA certificates of the PEM file at `path`.
fn roots_of_file(path: &str) -> Result<rustls::RootCertStore, String> {
    let certs = CertificateDer::pem_file_iter(path)
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|e| format!("sslrootcert {path}: {e}"))?;
    if certs.is_empty() {
        return Err(format!("sslrootcert {path} holds no certificate"));
    }
    let mut roots = rustls::RootCertStore::empty();
    for cert in certs {
        roots
            .add(cert)
            .map_err(|e| format!("sslrootcert {path}: {e}"))?;
    }
    Ok(roots)
}

/// Makes a TLS connector for each connection.
#[derive(Clone)]
pub(super) struct MakeRustlsConnect(tokio_rustls::TlsConnector);

impl MakeRustlsConnect {
    pub(super) fn new(config: Arc<rustls::ClientConfig>) -> Self {
        Self(tokio_rustls::TlsConnector::from(config))
    }
}

impl<S> MakeTlsConnect<S> for MakeRustlsConnect
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = RustlsStream<S>;
    type TlsConnect = RustlsConnect;
    type Error = io::Error;

    fn make_tls_connect(&mut self, domain: &str) -> io::Result<RustlsConnect> {
        let name = ServerName::try_from(domain.to_owned())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        Ok(RustlsConnect {
            connector: self.0.clone(),
            name,
        })
    }
}

/// The TLS connector for one connection, to the server `name`.
pub(super) struct RustlsConnect {
    connector: tokio_rustls::TlsConnector,
    name: ServerName<'static>,
}

impl<S> TlsConnect<S> for RustlsConnect
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = RustlsStream<S>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<RustlsStream<S>>> + Send>>;

    fn connect(self, stream: S) -> Self::Future {
        Box::pin(async move {
            let tls = self.connector.connect(self.name, stream).await?;
            Ok(RustlsStream(tls))
        })
    }
}

/// A TLS connection to the database server.
pub(super) struct RustlsStream<S>(tokio_rustls::client::TlsStream<S>);

impl<S: AsyncRead + AsyncWrite + Unpin> TlsStream for RustlsStream<S> {
    fn channel_binding(&self) -> ChannelBinding {
        ChannelBinding::none()
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for RustlsStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for RustlsStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

/// A self-signed CA certificate, for reading only.
#[cfg(test)]
pub(super) const TEST_CA: &str = "-----BEGIN CERTIFICATE-----
MIIBjTCCATOgAwIBAgIUeuA0SdsNrq90FD6lVm2QWRrqYcowCgYIKoZIzj0EAwIw
GzEZMBcGA1UEAwwQUG9ydGl0b3IgdGVzdCBDQTAgFw0yNjExMjYyMjA3MDNaGA8y
MTI2MTEwMjIyMDcwM1owGzEZMBcGA1UEAwwQUG9ydGl0b3IgdGVzdCBDQTBZMBMG
ByqGSM49AgEGCCqGSM49AwEHA0IABM1Mo2MvbwgktDcPXuuMKiL0zqlUwQCAsadk
joEXs+vVUXwR+7nMFW6ZttrjWGhPkT+c3cg27Kt5DXvOj0QlEQCjUzBRMB0GA1Ud
DgQWBBTSB6cr5ivTzao0pdbQPHxNZw2dJzAfBgNVHSMEGDAWgBTSB6cr5ivTzao0
pdbQPHxNZw2dJzAPBgNVHRMBAf8EBTADAQH/MAoGCCqGSM49BAMCA0gAMEUCIQC5
gJkZO21oQisNzjCuJNcag6CKtX9NUY3XcxYkTvp0lwIgbqoV/2lbKL2uKWLAGm1R
1mRKuSHqf8NkHRC4SNz6ZEw=
-----END CERTIFICATE-----
";

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// As libpq: the CAs of an `sslrootcert` file are the only ones
    /// trusted, not added to the system's.
    #[test]
    fn a_root_cert_file_is_the_only_ca_trusted() {
        let dir = crate::testing::TempDir::new("postgres-tls-roots");
        let path = dir.path().join("ca.pem");
        std::fs::write(&path, TEST_CA).unwrap();
        let roots = roots_of_file(path.to_str().unwrap()).unwrap();
        assert_eq!(roots.len(), 1);
    }

    #[test]
    fn a_root_cert_file_without_a_certificate_is_refused() {
        let dir = crate::testing::TempDir::new("postgres-tls-empty");
        let path = dir.path().join("empty.pem");
        std::fs::write(&path, "not a certificate\n").unwrap();
        let err = roots_of_file(path.to_str().unwrap()).unwrap_err();
        assert!(err.contains("holds no certificate"), "{err}");
        let missing = dir.path().join("missing.pem");
        let err = client_config(Some(missing.to_str().unwrap())).unwrap_err();
        assert!(err.starts_with("sslrootcert "), "{err}");
    }
}
