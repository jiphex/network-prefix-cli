//! Special-purpose address registries, so the tool can say "careful, that's
//! documentation space" before somebody numbers a live network out of it.

use ipnet::IpNet;

pub struct Special {
    pub prefix: &'static str,
    pub name: &'static str,
    pub rfc: &'static str,
    /// Prefixes you should not be assigning to real infrastructure.
    pub caution: bool,
}

const SPECIALS: &[Special] = &[
    // ---- IPv4 -----------------------------------------------------------
    Special {
        prefix: "0.0.0.0/8",
        name: "This network",
        rfc: "RFC 1122",
        caution: true,
    },
    Special {
        prefix: "0.0.0.0/32",
        name: "Unspecified / default route",
        rfc: "RFC 1122",
        caution: true,
    },
    Special {
        prefix: "10.0.0.0/8",
        name: "Private-Use",
        rfc: "RFC 1918",
        caution: false,
    },
    Special {
        prefix: "100.64.0.0/10",
        name: "Shared address space (CGNAT)",
        rfc: "RFC 6598",
        caution: false,
    },
    Special {
        prefix: "127.0.0.0/8",
        name: "Loopback",
        rfc: "RFC 1122",
        caution: true,
    },
    Special {
        prefix: "169.254.0.0/16",
        name: "Link-local",
        rfc: "RFC 3927",
        caution: true,
    },
    Special {
        prefix: "172.16.0.0/12",
        name: "Private-Use",
        rfc: "RFC 1918",
        caution: false,
    },
    Special {
        prefix: "192.0.0.0/24",
        name: "IETF protocol assignments",
        rfc: "RFC 6890",
        caution: true,
    },
    Special {
        prefix: "192.0.0.0/29",
        name: "DS-Lite",
        rfc: "RFC 6333",
        caution: true,
    },
    Special {
        prefix: "192.0.2.0/24",
        name: "Documentation (TEST-NET-1)",
        rfc: "RFC 5737",
        caution: true,
    },
    Special {
        prefix: "192.88.99.0/24",
        name: "6to4 relay anycast (deprecated)",
        rfc: "RFC 7526",
        caution: true,
    },
    Special {
        prefix: "192.168.0.0/16",
        name: "Private-Use",
        rfc: "RFC 1918",
        caution: false,
    },
    Special {
        prefix: "198.18.0.0/15",
        name: "Benchmarking",
        rfc: "RFC 2544",
        caution: true,
    },
    Special {
        prefix: "198.51.100.0/24",
        name: "Documentation (TEST-NET-2)",
        rfc: "RFC 5737",
        caution: true,
    },
    Special {
        prefix: "203.0.113.0/24",
        name: "Documentation (TEST-NET-3)",
        rfc: "RFC 5737",
        caution: true,
    },
    Special {
        prefix: "224.0.0.0/4",
        name: "Multicast",
        rfc: "RFC 5771",
        caution: true,
    },
    Special {
        prefix: "233.252.0.0/24",
        name: "MCAST-TEST-NET",
        rfc: "RFC 5771",
        caution: true,
    },
    Special {
        prefix: "240.0.0.0/4",
        name: "Reserved (former class E)",
        rfc: "RFC 1112",
        caution: true,
    },
    Special {
        prefix: "255.255.255.255/32",
        name: "Limited broadcast",
        rfc: "RFC 8190",
        caution: true,
    },
    // ---- IPv6 -----------------------------------------------------------
    Special {
        prefix: "::/128",
        name: "Unspecified",
        rfc: "RFC 4291",
        caution: true,
    },
    Special {
        prefix: "::1/128",
        name: "Loopback",
        rfc: "RFC 4291",
        caution: true,
    },
    Special {
        prefix: "::ffff:0:0/96",
        name: "IPv4-mapped",
        rfc: "RFC 4291",
        caution: true,
    },
    Special {
        prefix: "64:ff9b::/96",
        name: "NAT64 well-known prefix",
        rfc: "RFC 6052",
        caution: true,
    },
    Special {
        prefix: "64:ff9b:1::/48",
        name: "NAT64 local-use prefix",
        rfc: "RFC 8215",
        caution: false,
    },
    Special {
        prefix: "100::/64",
        name: "Discard-only",
        rfc: "RFC 6666",
        caution: true,
    },
    Special {
        prefix: "2000::/3",
        name: "Global unicast",
        rfc: "RFC 4291",
        caution: false,
    },
    Special {
        prefix: "2001::/32",
        name: "Teredo",
        rfc: "RFC 4380",
        caution: true,
    },
    Special {
        prefix: "2001:2::/48",
        name: "Benchmarking",
        rfc: "RFC 5180",
        caution: true,
    },
    Special {
        prefix: "2001:20::/28",
        name: "ORCHIDv2",
        rfc: "RFC 7343",
        caution: true,
    },
    Special {
        prefix: "2001:db8::/32",
        name: "Documentation",
        rfc: "RFC 3849",
        caution: true,
    },
    Special {
        prefix: "2002::/16",
        name: "6to4 (deprecated)",
        rfc: "RFC 7526",
        caution: true,
    },
    Special {
        prefix: "fc00::/7",
        name: "Unique local (ULA)",
        rfc: "RFC 4193",
        caution: false,
    },
    Special {
        prefix: "fe80::/10",
        name: "Link-local unicast",
        rfc: "RFC 4291",
        caution: true,
    },
    Special {
        prefix: "ff00::/8",
        name: "Multicast",
        rfc: "RFC 4291",
        caution: true,
    },
];

/// How a special range relates to the prefix under inspection.
pub enum Relation {
    /// The whole prefix sits inside the special range.
    Within,
    /// The prefix wholly contains the special range.
    Contains,
    /// Neither contains the other, but they share addresses.
    Overlaps,
}

pub struct Match {
    pub relation: Relation,
    pub net: IpNet,
    pub special: &'static Special,
}

impl Match {
    pub fn describe(&self) -> String {
        let verb = match self.relation {
            Relation::Within => "within",
            Relation::Contains => "contains",
            Relation::Overlaps => "overlaps",
        };
        format!(
            "{} {} - {} ({})",
            verb, self.net, self.special.name, self.special.rfc
        )
    }
}

/// Every special range that touches `net`, most specific first.
pub fn lookup(net: &IpNet) -> Vec<Match> {
    let mut out = Vec::new();
    for special in SPECIALS {
        let sp: IpNet = special.prefix.parse().expect("built-in prefix parses");
        if sp.addr().is_ipv4() != net.addr().is_ipv4() {
            continue;
        }
        let relation = if sp.contains(net) {
            Relation::Within
        } else if net.contains(&sp) {
            Relation::Contains
        } else if overlaps(&sp, net) {
            Relation::Overlaps
        } else {
            continue;
        };
        out.push(Match {
            relation,
            net: sp,
            special,
        });
    }
    // Most specific (longest special prefix) first.
    out.sort_by_key(|m| std::cmp::Reverse(m.net.prefix_len()));
    out
}

/// True when two prefixes share any address. Aligned prefixes either nest or
/// are disjoint, so this is just a mutual containment check.
fn overlaps(a: &IpNet, b: &IpNet) -> bool {
    a.contains(&b.network()) || b.contains(&a.network())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(s: &str) -> Vec<String> {
        lookup(&s.parse::<IpNet>().unwrap())
            .iter()
            .map(|m| m.special.name.to_string())
            .collect()
    }

    #[test]
    fn rfc1918_is_recognised() {
        assert_eq!(names("10.1.2.0/24"), vec!["Private-Use"]);
    }

    #[test]
    fn documentation_space_is_flagged() {
        let m = lookup(&"2001:db8::/52".parse().unwrap());
        assert!(
            m.iter()
                .any(|m| m.special.caution && m.special.rfc == "RFC 3849")
        );
    }

    #[test]
    fn nested_matches_are_most_specific_first() {
        // 2001::/64 is Teredo, which itself sits in 2000::/3.
        assert_eq!(names("2001::/64"), vec!["Teredo", "Global unicast"]);
    }

    #[test]
    fn a_containing_prefix_reports_contains() {
        let m = lookup(&"192.0.0.0/16".parse().unwrap());
        assert!(m.iter().any(|m| matches!(m.relation, Relation::Contains)));
    }

    #[test]
    fn families_do_not_cross() {
        assert!(names("::/0").iter().all(|n| n != "Private-Use"));
    }
}
