//! Reverse DNS delegation zones for a prefix.
//!
//! `in-addr.arpa` splits on octets and `ip6.arpa` on nibbles, so a prefix that
//! is not on one of those boundaries is not a zone - it is covered by several
//! of them. The `Reverse DNS` line in the report says as much and stops there;
//! this works out which zones those actually are, which is the thing you have
//! to go and create.
//!
//! IPv4 prefixes longer than a /24 have no zone of their own at all: the
//! octet below them is the last boundary there is. RFC 2317 delegates them
//! anyway, by pointing CNAMEs in the enclosing /24 at a made-up sub-zone, so
//! that is what gets reported rather than 2^n single-address zones.

use crate::num::Count;
use ipnet::IpNet;
use std::net::{IpAddr, Ipv6Addr};

/// How many bits one label of the reverse tree covers.
fn step(net: &IpNet) -> u8 {
    if net.addr().is_ipv4() { 8 } else { 4 }
}

/// The delegation boundary at or below a prefix: the shortest zone length that
/// still splits the prefix into whole zones.
pub fn natural_boundary(net: &IpNet) -> u8 {
    let step = step(net);
    net.prefix_len().div_ceil(step) * step
}

pub enum Zones {
    /// Cut into whole zones at `boundary`, each one a zone to create.
    Aligned {
        boundary: u8,
        count: Count,
        /// True when the prefix was already a zone in its own right.
        whole: bool,
    },
    /// RFC 2317: an IPv4 prefix longer than a /24, delegated out of the /24
    /// above it by CNAME.
    Classless {
        parent: String,
        zone: String,
        first: u8,
        last: u8,
    },
}

/// Work out the zones covering `net`, cut at `boundary` if one was asked for.
pub fn zones(net: &IpNet, boundary: Option<u8>) -> Result<Zones, String> {
    let step = step(net);
    let tree = if net.addr().is_ipv4() {
        "in-addr.arpa"
    } else {
        "ip6.arpa"
    };

    if let Some(b) = boundary {
        if b > net.max_prefix_len() {
            return Err(format!(
                "/{b} is not a valid length for {} ({net} tops out at /{})",
                crate::carve::family(net),
                net.max_prefix_len()
            ));
        }
        if b < net.prefix_len() {
            return Err(format!(
                ".{b} is shorter than {net}, so it is not a zone inside it"
            ));
        }
        if !b.is_multiple_of(step) {
            return Err(format!(
                ".{b} is not a delegation boundary: {tree} splits every {step} bits, \
                 so try .{} or .{}",
                b / step * step,
                b.div_ceil(step) * step
            ));
        }
        return Ok(Zones::Aligned {
            boundary: b,
            count: Count::pow2(u32::from(b - net.prefix_len())),
            whole: b == net.prefix_len(),
        });
    }

    // No boundary asked for, so use the natural one - except where IPv4 runs
    // out of boundaries entirely and RFC 2317 takes over.
    if net.addr().is_ipv4() && net.prefix_len() > 24 && net.prefix_len() < 32 {
        let IpAddr::V4(network) = net.network() else {
            unreachable!("checked ipv4")
        };
        let IpAddr::V4(broadcast) = net.broadcast() else {
            unreachable!("checked ipv4")
        };
        let first = network.octets()[3];
        let last = broadcast.octets()[3];
        let parent = name(&IpNet::new(net.network(), 24).expect("/24 is valid").trunc())
            .expect("a /24 is on an octet boundary");
        return Ok(Zones::Classless {
            // The RFC's own spelling. A slash is legal in a domain name as
            // long as nothing tries to parse it as one, and this label is
            // only ever an owner name.
            zone: format!("{first}/{}.{parent}", net.prefix_len()),
            parent,
            first,
            last,
        });
    }

    let boundary = natural_boundary(net);
    Ok(Zones::Aligned {
        boundary,
        count: Count::pow2(u32::from(boundary - net.prefix_len())),
        whole: boundary == net.prefix_len(),
    })
}

/// Every zone name, lazily: a `/32` cut into `/64` zones must not allocate
/// four billion strings before the first one is printed.
pub fn names(net: IpNet, boundary: u8) -> impl Iterator<Item = String> {
    net.subnets(boundary)
        .expect("boundary already checked against the prefix")
        .map(|sub| name(&sub).expect("a boundary length is always on a label boundary"))
}

/// The zone name for a prefix that sits on a label boundary.
pub fn name(net: &IpNet) -> Option<String> {
    let mut labels: Vec<String> = match net.network() {
        IpAddr::V4(a) => {
            if !net.prefix_len().is_multiple_of(8) {
                return None;
            }
            let take = usize::from(net.prefix_len() / 8);
            a.octets()[..take].iter().map(|o| o.to_string()).collect()
        }
        IpAddr::V6(a) => {
            if !net.prefix_len().is_multiple_of(4) {
                return None;
            }
            let take = usize::from(net.prefix_len() / 4);
            nibbles(a)[..take].iter().map(|n| n.to_string()).collect()
        }
    };
    labels.reverse();
    labels.push(
        if net.addr().is_ipv4() {
            "in-addr.arpa"
        } else {
            "ip6.arpa"
        }
        .into(),
    );
    Some(labels.join("."))
}

/// The 32 hex nibbles of an IPv6 address, most significant first.
fn nibbles(addr: Ipv6Addr) -> Vec<char> {
    addr.segments()
        .iter()
        .flat_map(|s| format!("{s:04x}").chars().collect::<Vec<_>>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(s: &str) -> IpNet {
        s.parse().unwrap()
    }

    fn aligned(s: &str, boundary: Option<u8>) -> (u8, u128, Vec<String>) {
        match zones(&net(s), boundary).unwrap() {
            Zones::Aligned {
                boundary, count, ..
            } => (
                boundary,
                count.as_u128().unwrap(),
                names(net(s), boundary).take(8).collect(),
            ),
            Zones::Classless { .. } => panic!("expected aligned zones for {s}"),
        }
    }

    #[test]
    fn a_prefix_on_a_boundary_is_one_zone() {
        let (b, n, names) = aligned("10.1.2.0/24", None);
        assert_eq!((b, n), (24, 1));
        assert_eq!(names, vec!["2.1.10.in-addr.arpa"]);

        let (b, n, names) = aligned("2001:db8::/32", None);
        assert_eq!((b, n), (32, 1));
        assert_eq!(names, vec!["8.b.d.0.1.0.0.2.ip6.arpa"]);
    }

    #[test]
    fn an_unaligned_ipv4_prefix_is_several_octet_zones() {
        let (b, n, names) = aligned("10.0.0.0/22", None);
        assert_eq!((b, n), (24, 4));
        assert_eq!(
            names,
            vec![
                "0.0.10.in-addr.arpa",
                "1.0.10.in-addr.arpa",
                "2.0.10.in-addr.arpa",
                "3.0.10.in-addr.arpa",
            ]
        );
    }

    #[test]
    fn an_unaligned_ipv6_prefix_is_several_nibble_zones() {
        let (b, n, names) = aligned("2001:db8::/50", None);
        assert_eq!((b, n), (52, 4));
        // A /52 is thirteen nibbles deep, so the zone carries thirteen
        // labels before ip6.arpa.
        assert_eq!(names[0], "0.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa");
        assert_eq!(names[3], "3.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa");
    }

    #[test]
    fn a_long_ipv4_prefix_is_delegated_classlessly() {
        let Zones::Classless {
            parent,
            zone,
            first,
            last,
        } = zones(&net("10.0.0.64/26"), None).unwrap()
        else {
            panic!("expected a classless delegation");
        };
        assert_eq!(parent, "0.0.10.in-addr.arpa");
        assert_eq!(zone, "64/26.0.0.10.in-addr.arpa");
        assert_eq!((first, last), (64, 127));
    }

    #[test]
    fn a_host_route_still_has_a_name() {
        // /32 and /128 are on a boundary, so they need no special case.
        let (b, n, names) = aligned("10.0.0.1/32", None);
        assert_eq!((b, n), (32, 1));
        assert_eq!(names, vec!["1.0.0.10.in-addr.arpa"]);
    }

    #[test]
    fn an_explicit_boundary_cuts_deeper() {
        let (b, n, names) = aligned("10.0.0.0/8", Some(16));
        assert_eq!((b, n), (16, 256));
        assert_eq!(names[0], "0.10.in-addr.arpa");
        assert_eq!(names[7], "7.10.in-addr.arpa");
    }

    #[test]
    fn an_explicit_boundary_must_be_on_a_label() {
        assert!(zones(&net("2001:db8::/32"), Some(50)).is_err());
        assert!(zones(&net("10.0.0.0/8"), Some(20)).is_err());
        // ... and must be inside the prefix.
        assert!(zones(&net("10.0.0.0/8"), Some(0)).is_err());
        assert!(zones(&net("10.0.0.0/8"), Some(64)).is_err());
    }

    #[test]
    fn zone_names_are_lazy() {
        // A /32 cut into /64s is 2^32 zones; the first must arrive at once.
        let mut it = names(net("2001:db8::/32"), 64);
        assert_eq!(
            it.next().unwrap(),
            "0.0.0.0.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa"
        );
    }
}
