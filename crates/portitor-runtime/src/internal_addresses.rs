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
//! refused when the host's address is internal ([`is_internal`]): not
//! globally reachable in the IANA special-purpose address registries
//! (loopback, private, link local, shared address space, "this host"
//! `0.0.0.0` and `::`, and others), multicast or broadcast; an IPv6
//! address that carries an IPv4 address counts as that IPv4 address.
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
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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
    /// IPv6 (`::ffff:10.0.0.1`) counts as the IPv4 address: it is the
    /// same host, as a dual-stack socket writes it.
    ///
    /// Only the mapped form is read here, while [`is_internal`] reads all
    /// the forms that carry an IPv4 address ([`ipv4_inside`]: also
    /// compatible, NAT64, 6to4, Teredo). On purpose: [`is_internal`]
    /// decides what is blocked, and reading more forms blocks more; this
    /// decides the exceptions, and reading more forms would allow more.
    /// A 6to4 or Teredo address names a network or a client behind the
    /// IPv4 address, not that host, so an exception for `10.1.0.0/16`
    /// should not let them through. An operator who wants a NAT64 form
    /// lists its IPv6 network (`64:ff9b::a01:0/112`).
    #[must_use]
    pub fn contains(&self, address: IpAddr) -> bool {
        match (self.address, as_ipv4_if_mapped(address)) {
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

/// Whether the first `prefix_length` bits of `network` and `address`
/// (their bytes, highest first) are the same.
fn same_prefix(network: &[u8], address: &[u8], prefix_length: u8) -> bool {
    let mut bits_left = usize::from(prefix_length);
    for (network_byte, address_byte) in network.iter().zip(address) {
        if bits_left == 0 {
            return true;
        }
        let bits = bits_left.min(8);
        // the `bits` highest bits of a byte, for example 0b1110_0000 for 3
        let mask: u8 = 0xff << (8 - bits);
        if network_byte & mask != address_byte & mask {
            return false;
        }
        bits_left -= bits;
    }
    true
}

/// `address`, with an IPv4 address mapped into IPv6 (`::ffff:a.b.c.d`)
/// written as IPv4; any other address as it is.
fn as_ipv4_if_mapped(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        IpAddr::V4(_) => address,
    }
}

/// An IPv4 network `first.second.third.fourth/prefix_length`, for the
/// table below.
const fn v4(first: u8, second: u8, third: u8, fourth: u8, prefix_length: u8) -> Network {
    Network {
        address: IpAddr::V4(Ipv4Addr::new(first, second, third, fourth)),
        prefix_length,
    }
}

/// An IPv6 network whose address starts with the four groups `first`,
/// `second`, `third` and `fourth` (the last four groups are zeros), for
/// the table below.
const fn v6(first: u16, second: u16, third: u16, fourth: u16, prefix_length: u8) -> Network {
    Network {
        address: IpAddr::V6(Ipv6Addr::new(first, second, third, fourth, 0, 0, 0, 0)),
        prefix_length,
    }
}

/// The internal IPv4 networks: every block of the IANA IPv4
/// Special-Purpose Address Registry
/// (<https://www.iana.org/assignments/iana-ipv4-special-registry>) that
/// is not globally reachable, and multicast (RFC 5771).
const INTERNAL_V4: [Network; 15] = [
    v4(0, 0, 0, 0, 8),       // "this network", with 0.0.0.0 "this host" (RFC 791)
    v4(10, 0, 0, 0, 8),      // private use (RFC 1918)
    v4(100, 64, 0, 0, 10),   // shared address space, carrier NAT (RFC 6598)
    v4(127, 0, 0, 0, 8),     // loopback (RFC 1122)
    v4(169, 254, 0, 0, 16),  // link local, cloud metadata (RFC 3927)
    v4(172, 16, 0, 0, 12),   // private use (RFC 1918)
    v4(192, 0, 0, 0, 24),    // IETF protocol assignments (RFC 6890)
    v4(192, 0, 2, 0, 24),    // documentation (RFC 5737)
    v4(192, 88, 99, 0, 24),  // deprecated 6to4 relay anycast (RFC 7526)
    v4(192, 168, 0, 0, 16),  // private use (RFC 1918)
    v4(198, 18, 0, 0, 15),   // benchmarking (RFC 2544)
    v4(198, 51, 100, 0, 24), // documentation (RFC 5737)
    v4(203, 0, 113, 0, 24),  // documentation (RFC 5737)
    v4(224, 0, 0, 0, 4),     // multicast (RFC 5771)
    v4(240, 0, 0, 0, 4),     // reserved, with the broadcast 255.255.255.255 (RFC 1112, RFC 919)
];

/// The internal IPv6 networks: every block of the IANA IPv6
/// Special-Purpose Address Registry
/// (<https://www.iana.org/assignments/iana-ipv6-special-registry>) that
/// is not globally reachable, the deprecated site-local block, and
/// multicast. Blocks that carry an IPv4 address are read by
/// [`ipv4_inside`] instead: loopback `::1`, for example, is the
/// IPv4-compatible form of `0.0.0.1`, in `0.0.0.0/8`.
const INTERNAL_V6: [Network; 12] = [
    v6(0, 0, 0, 0, 128),         // unspecified `::` (RFC 4291)
    v6(0x64, 0xff9b, 1, 0, 48),  // local-use IPv4/IPv6 translation (RFC 8215)
    v6(0x100, 0, 0, 0, 64),      // discard only (RFC 6666)
    v6(0x100, 0, 0, 1, 64),      // dummy prefix (RFC 9780)
    v6(0x2001, 2, 0, 0, 48),     // benchmarking (RFC 5180)
    v6(0x2001, 0xdb8, 0, 0, 32), // documentation (RFC 3849)
    v6(0x3fff, 0, 0, 0, 20),     // documentation (RFC 9637)
    v6(0x5f00, 0, 0, 0, 16),     // segment routing (RFC 9602)
    v6(0xfc00, 0, 0, 0, 7),      // unique local (RFC 4193)
    v6(0xfe80, 0, 0, 0, 10),     // link local (RFC 4291)
    v6(0xfec0, 0, 0, 0, 10),     // site local, deprecated (RFC 3879)
    v6(0xff00, 0, 0, 0, 8),      // multicast (RFC 4291)
];

/// The IPv4 address an IPv6 address carries, when it is one of the forms
/// that lead to an IPv4 host: IPv4-mapped `::ffff:a.b.c.d` (RFC 4291),
/// IPv4-compatible `::a.b.c.d` (RFC 4291, deprecated), NAT64
/// `64:ff9b::a.b.c.d` (RFC 6052), 6to4 `2002:aabb:ccdd::` (RFC 3056), and
/// Teredo `2001:0:…` (RFC 4380: the client's address, each bit inverted,
/// in the last 32 bits).
fn ipv4_inside(address: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = address.segments();
    let octets = address.octets();
    let last_32_bits = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
    if let Some(mapped) = address.to_ipv4_mapped() {
        return Some(mapped);
    }
    match segments {
        // IPv4-compatible `::a.b.c.d`, or NAT64 `64:ff9b::a.b.c.d`
        [0, 0, 0, 0, 0, 0, _, _] | [0x64, 0xff9b, 0, 0, 0, 0, _, _] => Some(last_32_bits),
        // 6to4: the IPv4 address follows `2002:`
        [0x2002, _, _, _, _, _, _, _] => {
            Some(Ipv4Addr::new(octets[2], octets[3], octets[4], octets[5]))
        }
        // Teredo: the client's address, each bit inverted
        [0x2001, 0, _, _, _, _, _, _] => Some(Ipv4Addr::from(!u32::from(last_32_bits))),
        _ => None,
    }
}

/// Whether `address` is internal: an address the IANA special-purpose
/// registries (IPv4 and IPv6, links at [`INTERNAL_V4`] and
/// [`INTERNAL_V6`]) mark as not globally reachable, such as loopback,
/// private, link local (with cloud metadata `169.254.169.254`), shared
/// address space (`100.64.0.0/10`, with Alibaba's metadata
/// `100.100.100.200`), "this host" (`0.0.0.0`, `::`), benchmarking and
/// documentation; or multicast, broadcast or site local (`fec0::/10`).
/// An IPv6 address that carries an IPv4 address ([`ipv4_inside`]:
/// mapped, compatible, NAT64, 6to4, Teredo) is internal when that IPv4
/// address is.
#[must_use]
pub fn is_internal(address: IpAddr) -> bool {
    let address = match address {
        IpAddr::V6(v6) => match ipv4_inside(v6) {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        IpAddr::V4(_) => address,
    };
    let table: &[Network] = match address {
        IpAddr::V4(_) => &INTERNAL_V4,
        IpAddr::V6(_) => &INTERNAL_V6,
    };
    table.iter().any(|network| network.contains(address))
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
    fn internal_addresses_are_those_not_globally_reachable() {
        for internal in [
            "127.0.0.1",
            "127.1.2.3",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "0.1.2.3",
            // shared address space: Kubernetes, carrier NAT, Alibaba metadata
            "100.64.0.1",
            "100.100.100.200",
            "100.127.255.255",
            "192.0.0.8",
            "198.18.0.1",
            "198.19.255.255",
            "192.0.2.1",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fd00::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "64:ff9b:1::1",
            // IPv6 forms that carry an internal IPv4 address
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
            "::169.254.169.254",
            "64:ff9b::127.0.0.1",
            "64:ff9b::a9fe:a9fe",
            "2002:7f00:1::",
            "2002:a9fe:a9fe::1",
            // Teredo, client 127.0.0.1 with each bit inverted
            "2001:0:4136:e378:8000:63bf:80ff:fffe",
        ] {
            assert!(is_internal(ip(internal)), "{internal}");
        }
        for public in [
            "8.8.8.8",
            "172.32.0.1",
            "100.63.255.255",
            "100.128.0.1",
            "198.20.0.1",
            "223.255.255.255",
            "2a00:1450:4001::1",
            "::ffff:8.8.8.8",
            "::8.8.8.8",
            "64:ff9b::8.8.8.8",
            "2002:808:808::",
            "fe00::1",
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

    /// An exception for an IPv4 network covers the mapped form only: a
    /// NAT64, 6to4 or Teredo address that carries one of its addresses is
    /// still refused, unless its IPv6 network is listed too.
    #[test]
    fn an_ipv4_exception_does_not_cover_other_ipv6_forms() {
        let on = InternalAddresses::from_vars(vars(&[
            (BLOCK_SETTING, "true"),
            (ALLOWED_SETTING, "10.1.0.0/16"),
        ]))
        .unwrap();
        assert!(on.allows("provider", ip("::ffff:10.1.0.5")));
        for other_form in [
            "::10.1.0.5",
            "64:ff9b::10.1.0.5",
            "2002:a01:5::1",
            "2001:0:4136:e378:8000:63bf:f5fe:fffa",
        ] {
            assert!(is_internal(ip(other_form)), "{other_form}");
            assert!(!on.allows("provider", ip(other_form)), "{other_form}");
        }
        let with_nat64 = InternalAddresses::from_vars(vars(&[
            (BLOCK_SETTING, "true"),
            (ALLOWED_SETTING, "10.1.0.0/16, 64:ff9b::a01:0/112"),
        ]))
        .unwrap();
        assert!(with_nat64.allows("provider", ip("64:ff9b::10.1.0.5")));
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
