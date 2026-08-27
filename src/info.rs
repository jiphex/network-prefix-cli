//! Everything worth knowing about a single prefix.

use crate::num::Count;
use crate::wellknown::{self, Match};
use crate::zones;
use ipnet::IpNet;
use std::net::{IpAddr, Ipv4Addr};

pub struct Info {
    /// The prefix as the user typed it, if it needed normalising.
    pub given: Option<String>,
    pub net: IpNet,
    pub addresses: Count,
    /// IPv4 only: usable host addresses and the range they span.
    pub hosts: Option<Hosts>,
    pub reverse: Reverse,
    pub specials: Vec<Match>,
}

pub struct Hosts {
    pub count: u64,
    pub first: IpAddr,
    pub last: IpAddr,
    /// RFC 3021 point-to-point, or a single host route.
    pub note: Option<&'static str>,
}

pub enum Reverse {
    Zone(String),
    /// Not on a delegation boundary; carries the reason.
    Unaligned(&'static str),
}

impl Info {
    pub fn new(input: &str, net: IpNet) -> Info {
        let trunc = net.trunc();
        let given = (trunc != net).then(|| input.to_string());
        let host_bits = u32::from(net.max_prefix_len() - net.prefix_len());
        Info {
            given,
            net: trunc,
            addresses: Count::pow2(host_bits),
            hosts: hosts(&trunc),
            reverse: reverse_zone(&trunc),
            specials: wellknown::lookup(&trunc),
        }
    }

    pub fn family(&self) -> &'static str {
        crate::carve::family(&self.net)
    }

    pub fn is_ipv4(&self) -> bool {
        self.net.addr().is_ipv4()
    }

    /// The last address in the prefix. For IPv4 this is the broadcast address.
    pub fn last(&self) -> IpAddr {
        self.net.broadcast()
    }

    /// Fully expanded IPv6 form, which is what you want when comparing
    /// prefixes by eye. `None` for IPv4, where the text form is already exact.
    pub fn expanded(&self) -> Option<String> {
        match self.net.network() {
            IpAddr::V6(a) => Some(
                a.segments()
                    .iter()
                    .map(|s| format!("{s:04x}"))
                    .collect::<Vec<_>>()
                    .join(":"),
            ),
            IpAddr::V4(_) => None,
        }
    }

    /// Number of subnets of `len` this prefix holds, if `len` is longer.
    pub fn subnet_count(&self, len: u8) -> Option<Count> {
        (len >= self.net.prefix_len() && len <= self.net.max_prefix_len())
            .then(|| Count::pow2(u32::from(len - self.net.prefix_len())))
    }

    /// Lengths worth quoting counts for, given the prefix's own size.
    pub fn common_splits(&self) -> Vec<u8> {
        let len = self.net.prefix_len();
        let candidates: &[u8] = if self.is_ipv4() {
            &[24, 26, 28, 30, 31]
        } else {
            &[48, 56, 64, 80, 96, 112, 128]
        };
        candidates
            .iter()
            .copied()
            .filter(|c| *c > len)
            .take(3)
            .collect()
    }

    pub fn cautions(&self) -> Vec<&Match> {
        self.specials
            .iter()
            .filter(|m| m.special.caution && matches!(m.relation, wellknown::Relation::Within))
            .collect()
    }
}

fn hosts(net: &IpNet) -> Option<Hosts> {
    let IpAddr::V4(network) = net.network() else {
        return None;
    };
    let IpAddr::V4(broadcast) = net.broadcast() else {
        return None;
    };
    let (first, last, count, note) = match net.prefix_len() {
        32 => (network, network, 1, Some("host route - a single address")),
        31 => (
            network,
            broadcast,
            2,
            Some("point-to-point link, both addresses usable (RFC 3021)"),
        ),
        _ => (
            Ipv4Addr::from(u32::from(network) + 1),
            Ipv4Addr::from(u32::from(broadcast) - 1),
            u64::from(u32::from(broadcast) - u32::from(network) - 1),
            None,
        ),
    };
    Some(Hosts {
        count,
        first: IpAddr::V4(first),
        last: IpAddr::V4(last),
        note,
    })
}

fn reverse_zone(net: &IpNet) -> Reverse {
    match zones::name(net) {
        Some(zone) => Reverse::Zone(zone),
        None if net.addr().is_ipv4() => {
            Reverse::Unaligned("not on an octet boundary - needs RFC 2317 classless delegation")
        }
        None => Reverse::Unaligned("not on a nibble boundary - no clean ip6.arpa zone"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(s: &str) -> Info {
        Info::new(s, s.parse().unwrap())
    }

    fn zone(s: &str) -> String {
        match info(s).reverse {
            Reverse::Zone(z) => z,
            Reverse::Unaligned(_) => panic!("expected an aligned zone for {s}"),
        }
    }

    #[test]
    fn counts_addresses() {
        assert_eq!(info("10.0.0.0/24").addresses.as_u128(), Some(256));
        assert_eq!(
            info("2001:db8::/64").addresses.digits(),
            "18446744073709551616"
        );
        assert_eq!(info("::/0").addresses.as_u128(), None);
    }

    #[test]
    fn usable_hosts_follow_the_ipv4_rules() {
        let h = info("10.0.0.0/24").hosts.unwrap();
        assert_eq!(h.count, 254);
        assert_eq!(h.first.to_string(), "10.0.0.1");
        assert_eq!(h.last.to_string(), "10.0.0.254");

        assert_eq!(info("10.0.0.0/31").hosts.unwrap().count, 2);
        assert_eq!(info("10.0.0.1/32").hosts.unwrap().count, 1);
        assert_eq!(info("10.0.0.0/30").hosts.unwrap().count, 2);
        assert!(info("2001:db8::/64").hosts.is_none());
    }

    #[test]
    fn reverse_zones() {
        assert_eq!(zone("10.1.2.0/24"), "2.1.10.in-addr.arpa");
        assert_eq!(zone("10.0.0.0/8"), "10.in-addr.arpa");
        assert_eq!(zone("0.0.0.0/0"), "in-addr.arpa");
        assert_eq!(zone("2001:db8::/32"), "8.b.d.0.1.0.0.2.ip6.arpa");
        assert!(matches!(info("10.0.0.0/26").reverse, Reverse::Unaligned(_)));
        assert!(matches!(
            info("2001:db8::/63").reverse,
            Reverse::Unaligned(_)
        ));
    }

    #[test]
    fn host_bits_are_normalised_and_reported() {
        let i = info("10.0.0.77/24");
        assert_eq!(i.net.to_string(), "10.0.0.0/24");
        assert_eq!(i.given.as_deref(), Some("10.0.0.77/24"));
        assert!(info("10.0.0.0/24").given.is_none());
    }

    #[test]
    fn expanded_form_is_ipv6_only() {
        assert_eq!(
            info("2001:db8::/48").expanded().unwrap(),
            "2001:0db8:0000:0000:0000:0000:0000:0000"
        );
        assert!(info("10.0.0.0/8").expanded().is_none());
    }

    #[test]
    fn documentation_space_raises_a_caution() {
        assert!(!info("2001:db8::/52").cautions().is_empty());
        assert!(info("10.0.0.0/8").cautions().is_empty());
    }

    #[test]
    fn common_splits_are_longer_than_the_prefix() {
        assert_eq!(info("2001:db8::/52").common_splits(), vec![56, 64, 80]);
        assert_eq!(info("10.0.0.0/24").common_splits(), vec![26, 28, 30]);
        assert!(info("10.0.0.0/32").common_splits().is_empty());
    }
}
