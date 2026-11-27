// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The rule for partner-chosen URLs that lead inside a network (R-17,
//! from 1.4; ADR 0016, amendment of 1.4).
//!
//! A partner chooses some of the URLs a Portitor process calls: a
//! consumer's `callbackAddress`, a caller's DID document and credential
//! service, a status list, a push `endpoint`, a holder's credential
//! service. With `PORTITOR_BLOCK_INTERNAL_ADDRESSES=true`, such a call is
//! refused when the host's address is internal ([`is_internal`]):
//! loopback, private or link-local, or "this host" (`0.0.0.0`, `::`).
//! `PORTITOR_INTERNAL_ADDRESSES_ALLOWED` names the exceptions: host names
//! (compared without letter case), IP addresses and IP networks
//! (`10.1.0.0/16`).
//!
//! Off by default in 1.x: every demo and Compose setup calls its partners
//! on private addresses, and a block that is on by default would be a
//! change of behaviour after the freeze (STABILITY). On by default in
//! 2.0 (ROADMAP, "For 2.0").
//!
//! URLs the operator chooses (its own STS, its data sources, its object
//! storage, the bundle URL, the database) are never checked here.

use std::ffi::OsString;
use std::net::IpAddr;

use crate::settings;

/// The switch that turns the block on.
pub const BLOCK_SETTING: &str = "PORTITOR_BLOCK_INTERNAL_ADDRESSES";

/// The list of exceptions to the block.
pub const ALLOWED_SETTING: &str = "PORTITOR_INTERNAL_ADDRESSES_ALLOWED";

/// An IP network written `address/prefix length`, or one address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Network {
    address: IpAddr,
    prefix_length: u8,
}

impl Network {
    /// Reads `10.1.0.0/16`, `fd00::/8`, or one address (`10.1.2.3`).
    /// `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (address, prefix_length) = match text.split_once('/') {
            Some((address, length)) => (address, Some(length)),
            None => (text, None),
        };
        let address: IpAddr = address.parse().ok()?;
        let longest: u8 = if address.is_ipv4() { 32 } else { 128 };
        let prefix_length = match prefix_length {
            Some(length) => length.parse::<u8>().ok().filter(|l| *l <= longest)?,
            None => longest,
        };
        Some(Self {
            address,
            prefix_length,
        })
    }

    /// Whether `address` is in this network. An IPv4 address mapped into
    /// IPv6 (`::ffff:10.0.0.1`) counts as the IPv4 address.
    #[must_use]
    pub fn contains(&self, address: IpAddr) -> bool {
        match (self.address, plain(address)) {
            (IpAddr::V4(network), IpAddr::V4(address)) => {
                same_prefix(&network.octets(), &address.octets(), self.prefix_length)
            }
            (IpAddr::V6(network), IpAddr::V6(address)) => {
                same_prefix(&network.octets(), &address.octets(), self.prefix_length)
            }
            _ => false,
        }
    }
}

/// Whether the first `length` bits of `a` and `b` are the same.
fn same_prefix(a: &[u8], b: &[u8], length: u8) -> bool {
    let mut bits_left = usize::from(length);
    for (byte_a, byte_b) in a.iter().zip(b) {
        if bits_left == 0 {
            return true;
        }
        let bits = bits_left.min(8);
        // the `bits` highest bits of a byte, for example 0b1110_0000 for 3
        let mask: u8 = 0xff << (8 - bits);
        if byte_a & mask != byte_b & mask {
            return false;
        }
        bits_left -= bits;
    }
    true
}

/// `address`, with an IPv4 address mapped into IPv6 written as IPv4.
fn plain(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        IpAddr::V4(_) => address,
    }
}

/// Whether `address` is internal: loopback (`127.0.0.0/8`, `::1`),
/// private (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`,
/// `fc00::/7`), link-local (`169.254.0.0/16`, `fe80::/10`), or "this
/// host" (`0.0.0.0`, `::`), which reaches the local machine.
#[must_use]
pub fn is_internal(address: IpAddr) -> bool {
    match plain(address) {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_unspecified()
        }
    }
}

/// The two settings, read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InternalAddresses {
    /// `PORTITOR_BLOCK_INTERNAL_ADDRESSES`: whether internal addresses
    /// are refused for partner-chosen URLs.
    pub block: bool,
    /// Host names of `PORTITOR_INTERNAL_ADDRESSES_ALLOWED`, in lower case.
    pub allowed_hosts: Vec<String>,
    /// IP addresses and networks of `PORTITOR_INTERNAL_ADDRESSES_ALLOWED`.
    pub allowed_networks: Vec<Network>,
}

impl InternalAddresses {
    /// Reads the two settings from the process environment, at start.
    ///
    /// # Errors
    ///
    /// As [`InternalAddresses::from_vars`].
    pub fn from_env() -> Result<Self, String> {
        Self::from_vars(|name| std::env::var_os(name))
    }

    /// Reads the two settings with `var`.
    ///
    /// # Errors
    ///
    /// The switch is not `true` or `false`, the list is set but names
    /// nothing, or an item looks like an address or network but is not
    /// one (`10.0.0.0/33`). The text starts with the setting's name.
    pub fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Result<Self, String> {
        let block = settings::switch(&var, BLOCK_SETTING)
            .map_err(|problem| format!("{BLOCK_SETTING} {problem}"))?;
        let items = settings::list(&var, ALLOWED_SETTING)
            .map_err(|problem| format!("{ALLOWED_SETTING} {problem}"))?
            .unwrap_or_default();
        let mut allowed = Self {
            block,
            ..Self::default()
        };
        for item in items {
            if let Some(network) = Network::parse(&item) {
                allowed.allowed_networks.push(network);
            } else if looks_like_an_address(&item) {
                return Err(format!(
                    "{ALLOWED_SETTING}: {item:?} is not an IP address or network such as 10.1.0.0/16"
                ));
            } else {
                allowed.allowed_hosts.push(item.to_ascii_lowercase());
            }
        }
        Ok(allowed)
    }

    /// Says in the log, at start, whether the block is on, and how many
    /// exceptions it has.
    pub fn log_at_start(&self) {
        if self.block {
            tracing::info!(
                allowed_hosts = self.allowed_hosts.len(),
                allowed_networks = self.allowed_networks.len(),
                "internal addresses are refused for URLs partners choose ({BLOCK_SETTING})"
            );
        }
    }

    /// Whether a call to `host` (as written in the URL), which resolved
    /// to `address`, may go ahead: the block is off, the address is not
    /// internal, or the host or the address is an exception.
    #[must_use]
    pub fn allows(&self, host: &str, address: IpAddr) -> bool {
        if !self.block || !is_internal(address) {
            return true;
        }
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if self
            .allowed_hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
        {
            return true;
        }
        self.allowed_networks
            .iter()
            .any(|network| network.contains(address))
    }
}

/// Whether `item` was meant as an address: it has a `/`, or it is made
/// of digits and dots only, or it has a `:` (IPv6). A host name has none
/// of these.
fn looks_like_an_address(item: &str) -> bool {
    item.contains(['/', ':']) || item.chars().all(|c| c.is_ascii_digit() || c == '.')
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testing::vars;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn internal_addresses_are_loopback_private_link_local_and_this_host() {
        for internal in [
            "127.0.0.1",
            "127.1.2.3",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "::1",
            "::",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
        ] {
            assert!(is_internal(ip(internal)), "{internal}");
        }
        for public in [
            "8.8.8.8",
            "172.32.0.1",
            "100.64.0.1",
            "2001:db8::1",
            "::ffff:8.8.8.8",
        ] {
            assert!(!is_internal(ip(public)), "{public}");
        }
    }

    #[test]
    fn networks_contain_their_addresses() {
        let network = Network::parse("10.1.0.0/16").unwrap();
        assert!(network.contains(ip("10.1.200.3")));
        assert!(network.contains(ip("::ffff:10.1.0.1")));
        assert!(!network.contains(ip("10.2.0.1")));
        assert!(!network.contains(ip("fd00::1")));
        let odd = Network::parse("172.16.0.0/12").unwrap();
        assert!(odd.contains(ip("172.31.0.1")));
        assert!(!odd.contains(ip("172.32.0.1")));
        let one = Network::parse("fd00::5").unwrap();
        assert!(one.contains(ip("fd00::5")));
        assert!(!one.contains(ip("fd00::6")));
        assert!(
            Network::parse("0.0.0.0/0")
                .unwrap()
                .contains(ip("10.0.0.1"))
        );
        for wrong in [
            "10.0.0.0/33",
            "fd00::/129",
            "10.0.0/8",
            "host",
            "10.0.0.0/x",
        ] {
            assert_eq!(Network::parse(wrong), None, "{wrong}");
        }
    }

    #[test]
    fn off_by_default_and_exceptions_when_on() {
        let off = InternalAddresses::from_vars(vars(&[])).unwrap();
        assert!(!off.block);
        assert!(off.allows("localhost", ip("127.0.0.1")));

        let on = InternalAddresses::from_vars(vars(&[
            (BLOCK_SETTING, "true"),
            (ALLOWED_SETTING, "Provider, 10.1.0.0/16, fd00::7"),
        ]))
        .unwrap();
        assert_eq!(on.allowed_hosts, ["provider"]);
        assert!(on.allows("example.com", ip("93.184.215.14")));
        assert!(!on.allows("localhost", ip("127.0.0.1")));
        assert!(!on.allows("metadata", ip("169.254.169.254")));
        assert!(on.allows("PROVIDER", ip("10.9.0.2")));
        assert!(on.allows("consumer", ip("10.1.4.4")));
        assert!(on.allows("[fd00::7]", ip("fd00::7")));
        assert!(!on.allows("[fd00::8]", ip("fd00::8")));
    }

    #[test]
    fn a_wrong_value_is_refused_with_the_name() {
        for (name, value) in [
            (BLOCK_SETTING, "yes"),
            (ALLOWED_SETTING, " , "),
            (ALLOWED_SETTING, "10.0.0.0/33"),
            (ALLOWED_SETTING, "300.1.1.1"),
        ] {
            let error = InternalAddresses::from_vars(vars(&[(name, value)])).unwrap_err();
            assert!(error.starts_with(name), "{value}: {error}");
        }
    }
}
