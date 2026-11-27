// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! How an outbound call opens its TCP connection, with the rule for
//! internal addresses (R-17; ADR 0016, amended in 1.4).
//!
//! hyper's client asks a "connector" for each new connection. Ours,
//! [`CheckedConnector`], is hyper's own `HttpConnector` with one change:
//! the host name is resolved by [`CheckedResolver`], which keeps only the
//! addresses the rule allows ([`InternalAddresses::allows`]). The
//! connection then goes to one of those addresses, so the address
//! checked is the address called: there is no second DNS lookup that
//! could answer differently. A URL whose host is an IP address is never
//! resolved, so [`CheckedConnector`] checks that address itself.
//!
//! With the block off (the default in 1.x) every address is allowed and
//! nothing changes. Clients for URLs the operator chooses get the rule
//! switched off ([`InternalAddresses::default`]), whatever the setting.

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use hyper::Uri;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::connect::dns::Name;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

use crate::internal_addresses::{ALLOWED_SETTING, BLOCK_SETTING, InternalAddresses};

/// Any error, as hyper's connectors pass them on.
type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A task that finishes later with a `T` or an error; what the two
/// services below return (`Service::Future`).
type Later<T> = Pin<Box<dyn Future<Output = Result<T, BoxError>> + Send>>;

/// Why a call was not made: its host is internal, and the rule refuses
/// it. The text names the setting that would allow it. It is for our
/// own operator (the log, the management API), never for a partner: it
/// names an address inside our network. The data plane's push, for
/// example, tells the consumer only that its endpoint is not allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalAddressRefused {
    /// The host, as written in the URL.
    pub host: String,
    /// The internal address it is, or resolved to.
    pub address: IpAddr,
}

impl std::fmt::Display for InternalAddressRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{host} leads to the internal address {address}, refused because a partner chose \
             this URL and {BLOCK_SETTING} is true; to allow it, add {host} or {address} to \
             {ALLOWED_SETTING}",
            host = self.host,
            address = self.address
        )
    }
}

impl std::error::Error for InternalAddressRefused {}

/// The refusal in `error` or in one of its causes, if there is one.
/// hyper wraps a connector's error in its own errors; this finds it.
#[must_use]
pub fn refusal_in<'a>(
    error: &'a (dyn std::error::Error + 'static),
) -> Option<&'a InternalAddressRefused> {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(error) = current {
        if let Some(refused) = error.downcast_ref::<InternalAddressRefused>() {
            return Some(refused);
        }
        current = error.source();
    }
    None
}

/// Resolves host names with the system's resolver, as hyper's own
/// resolver does, and keeps only the addresses `rule` allows.
#[derive(Debug, Clone)]
pub struct CheckedResolver {
    rule: Arc<InternalAddresses>,
}

/// hyper calls a resolver as a `tower` service: `call` with a host name
/// gives, later, the addresses to try.
impl tower_service::Service<Name> for CheckedResolver {
    type Response = std::vec::IntoIter<SocketAddr>;
    type Error = BoxError;
    type Future = Later<Self::Response>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // always ready: each call does its own lookup
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, name: Name) -> Self::Future {
        let rule = Arc::clone(&self.rule);
        Box::pin(async move {
            let host = name.as_str().to_owned();
            // port 0: hyper puts the URL's port on each address
            let found = tokio::net::lookup_host((host.as_str(), 0)).await?;
            match allowed_addresses(&rule, &host, found) {
                Ok(allowed) => Ok(allowed.into_iter()),
                Err(refused) => Err(refused.into()),
            }
        })
    }
}

/// The addresses of `found` (what `host` resolved to) that `rule`
/// allows. Refused when it allows none of them while one of them is
/// internal. An empty list stays empty: hyper reports that itself.
fn allowed_addresses(
    rule: &InternalAddresses,
    host: &str,
    found: impl Iterator<Item = SocketAddr>,
) -> Result<Vec<SocketAddr>, InternalAddressRefused> {
    let mut allowed = Vec::new();
    let mut first_refused = None;
    for address in found {
        if rule.allows(host, address.ip()) {
            allowed.push(address);
        } else if first_refused.is_none() {
            first_refused = Some(address.ip());
        }
    }
    match first_refused {
        Some(address) if allowed.is_empty() => Err(InternalAddressRefused {
            host: host.to_owned(),
            address,
        }),
        _ => Ok(allowed),
    }
}

/// hyper's TCP connector with the rule for internal addresses: host
/// names go through [`CheckedResolver`], and a host that is an IP
/// address is checked here.
#[derive(Debug, Clone)]
pub struct CheckedConnector {
    tcp: HttpConnector<CheckedResolver>,
    rule: Arc<InternalAddresses>,
}

impl CheckedConnector {
    /// A connector that applies `rule`; `connect_timeout`: how long a
    /// connection may take to open, `None` for as long as the system
    /// waits. Plain `http://` and `https://` URLs both pass through it:
    /// TLS is added on top (`hyper_rustls`).
    #[must_use]
    pub fn new(rule: InternalAddresses, connect_timeout: Option<std::time::Duration>) -> Self {
        let rule = Arc::new(rule);
        let resolver = CheckedResolver {
            rule: Arc::clone(&rule),
        };
        let mut tcp = HttpConnector::new_with_resolver(resolver);
        // what `hyper_rustls`' builder sets up by default: the TLS layer
        // above decides about `https://`
        tcp.enforce_http(false);
        tcp.set_connect_timeout(connect_timeout);
        Self { tcp, rule }
    }
}

/// hyper calls a connector as a `tower` service: `call` with the URL
/// gives, later, the open TCP connection.
impl tower_service::Service<Uri> for CheckedConnector {
    type Response = TokioIo<TcpStream>;
    type Error = BoxError;
    type Future = Later<Self::Response>;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.tcp.poll_ready(context).map_err(Into::into)
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        // hyper does not resolve an IP address written in the URL, so the
        // resolver never sees it: it is checked here
        if let Some(host) = uri.host() {
            let bare = host.trim_start_matches('[').trim_end_matches(']');
            if let Ok(address) = bare.parse::<IpAddr>()
                && !self.rule.allows(host, address)
            {
                let refused = InternalAddressRefused {
                    host: host.to_owned(),
                    address,
                };
                return Box::pin(async move { Err(refused.into()) });
            }
        }
        let connecting = self.tcp.call(uri);
        Box::pin(async move { connecting.await.map_err(Into::into) })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testing::vars;
    use tower_service::Service as _;

    fn blocking(allowed: &str) -> InternalAddresses {
        InternalAddresses::from_vars(vars(&[(BLOCK_SETTING, "true"), (ALLOWED_SETTING, allowed)]))
            .unwrap()
    }

    fn at(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    #[test]
    fn only_allowed_addresses_are_kept() {
        let rule = blocking("10.1.0.0/16");
        let found = [at("127.0.0.1:0"), at("93.184.215.14:0"), at("10.1.0.5:0")];
        let kept = allowed_addresses(&rule, "mixed.example", found.into_iter()).unwrap();
        assert_eq!(kept, [at("93.184.215.14:0"), at("10.1.0.5:0")]);

        let refused = allowed_addresses(&rule, "inside.example", [at("192.168.1.1:0")].into_iter())
            .unwrap_err();
        assert_eq!(refused.address, "192.168.1.1".parse::<IpAddr>().unwrap());
        let text = refused.to_string();
        assert!(text.contains("inside.example"), "{text}");
        assert!(text.contains(ALLOWED_SETTING), "{text}");

        assert!(
            allowed_addresses(&rule, "none.example", std::iter::empty())
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn localhost_resolves_to_a_refusal_and_its_name_allows_it() {
        let mut resolver = CheckedResolver {
            rule: Arc::new(blocking("10.1.0.0/16")),
        };
        let error = resolver
            .call("localhost".parse().unwrap())
            .await
            .unwrap_err();
        assert!(refusal_in(error.as_ref()).is_some(), "{error}");

        let mut allowed = CheckedResolver {
            rule: Arc::new(blocking("localhost")),
        };
        let addresses: Vec<_> = allowed
            .call("localhost".parse().unwrap())
            .await
            .unwrap()
            .collect();
        assert!(!addresses.is_empty());
    }

    #[tokio::test]
    async fn an_address_in_the_url_is_checked_without_a_lookup() {
        let mut connector = CheckedConnector::new(blocking("localhost"), None);
        for url in [
            "http://127.0.0.1:9/",
            "http://[::1]:9/",
            "http://0.0.0.0:9/",
        ] {
            let error = connector.call(url.parse().unwrap()).await.unwrap_err();
            let refused = refusal_in(error.as_ref()).unwrap();
            assert!(refused.to_string().contains(ALLOWED_SETTING), "{url}");
        }
    }
}
