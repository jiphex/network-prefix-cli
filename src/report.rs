//! Turns a prefix plus a list of operators into everything we want to print.
//!
//! Operators are grouped rather than applied strictly left to right: all the
//! carve operators feed one allocator run, and any split then describes what
//! that run left behind. Splitting the remainder is what a planner actually
//! wants to know - "after those allocations, how many /64s have I still got?"

use crate::carve::{self, Plan, Request};
use crate::info::Info;
use crate::num::Count;
use crate::ops::{Op, Target};
use ipnet::IpNet;
use std::net::IpAddr;

pub struct Report {
    pub info: Info,
    pub supernets: Vec<Supernet>,
    pub lookups: Vec<Lookup>,
    pub carve: Option<Plan>,
    pub splits: Vec<Split>,
}

pub struct Supernet {
    pub net: IpNet,
    /// How many prefixes the size of the original fit inside it.
    pub siblings: Count,
}

pub struct Lookup {
    pub target: Target,
    pub inside: bool,
    /// For each requested split length, the subnet the target lands in and
    /// its index within the split.
    pub positions: Vec<(u8, IpNet, u128)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Source {
    /// Splitting the prefix itself.
    Whole,
    /// Splitting whatever the carve operators left free.
    Remainder,
}

pub struct Split {
    pub len: u8,
    pub source: Source,
    /// The blocks actually being divided, largest first.
    pub blocks: Vec<IpNet>,
    /// Free blocks already too small to hold a subnet of this length.
    pub too_small: usize,
}

impl Split {
    pub fn counts(&self) -> Vec<Count> {
        self.blocks
            .iter()
            .map(|b| Count::pow2(u32::from(self.len - b.prefix_len())))
            .collect()
    }

    /// Every subnet, lazily - a /0 split into /128s must not allocate.
    pub fn subnets(&self) -> impl Iterator<Item = IpNet> + '_ {
        let len = self.len;
        self.blocks
            .iter()
            .flat_map(move |b| b.subnets(len).expect("block is short enough"))
    }

    pub fn first(&self) -> Option<IpNet> {
        self.blocks.first().map(|b| first_subnet(*b, self.len))
    }

    pub fn last(&self) -> Option<IpNet> {
        self.blocks.last().map(|b| last_subnet(*b, self.len))
    }
}

pub fn build(input: &str, net: IpNet, ops: &[Op]) -> Result<Report, String> {
    let info = Info::new(input, net);
    let net = info.net;
    let max = net.max_prefix_len();

    let mut requests = Vec::new();
    let mut split_lens = Vec::new();
    let mut supernets = Vec::new();
    let mut lookups = Vec::new();

    for op in ops {
        match op {
            Op::Split(len) => {
                check_len(*len, max, net)?;
                if *len < net.prefix_len() {
                    return Err(format!(
                        "/{len} is larger than {net}, so it cannot be a subnet of it \
                         (did you mean +{len}?)"
                    ));
                }
                if !split_lens.contains(len) {
                    split_lens.push(*len);
                }
            }
            Op::Supernet(len) => {
                check_len(*len, max, net)?;
                if *len >= net.prefix_len() {
                    return Err(format!(
                        "+{len} is not shorter than {net}, so it is not a supernet of it \
                         (did you mean /{len}?)"
                    ));
                }
                let sup = IpNet::new(net.addr(), *len)
                    .expect("length already checked")
                    .trunc();
                supernets.push(Supernet {
                    net: sup,
                    siblings: Count::pow2(u32::from(net.prefix_len() - len)),
                });
            }
            Op::Carve { len, count } => {
                if *count > carve::MAX_REQUEST_COUNT {
                    return Err(format!(
                        "{count} subnets is more than this carves in one go \
                         (limit {}); use /{len} to describe a split that size instead",
                        carve::MAX_REQUEST_COUNT
                    ));
                }
                // Expanded so each subnet gets its own line in the output.
                for _ in 0..*count {
                    requests.push(Request::Floating {
                        len: *len,
                        count: 1,
                    });
                }
            }
            Op::Exclude(target) => requests.push(Request::Fixed(*target)),
            Op::Contains(target) => {
                if target.is_ipv4() != net.addr().is_ipv4() {
                    return Err(format!(
                        "{target} is {} but {net} is {}",
                        if target.is_ipv4() { "IPv4" } else { "IPv6" },
                        carve::family(&net)
                    ));
                }
                lookups.push(Lookup {
                    target: target.clone(),
                    inside: match target {
                        Target::Addr(a) => net.contains(a),
                        Target::Net(n) => net.contains(n),
                    },
                    positions: Vec::new(),
                });
            }
        }
    }

    let plan = (!requests.is_empty()).then(|| carve::plan(net, &requests));

    let splits = split_lens
        .iter()
        .map(|len| match &plan {
            Some(plan) => {
                let blocks: Vec<IpNet> = plan
                    .free
                    .iter()
                    .copied()
                    .filter(|b| b.prefix_len() <= *len)
                    .collect();
                Split {
                    len: *len,
                    source: Source::Remainder,
                    too_small: plan.free.len() - blocks.len(),
                    blocks,
                }
            }
            None => Split {
                len: *len,
                source: Source::Whole,
                blocks: vec![net],
                too_small: 0,
            },
        })
        .collect::<Vec<_>>();

    for lookup in &mut lookups {
        if !lookup.inside {
            continue;
        }
        let addr = match &lookup.target {
            Target::Addr(a) => *a,
            Target::Net(n) => n.network(),
        };
        for len in &split_lens {
            let sub = IpNet::new(addr, *len)
                .expect("length already checked")
                .trunc();
            lookup.positions.push((*len, sub, index_of(net, sub)));
        }
    }

    Ok(Report {
        info,
        supernets,
        lookups,
        carve: plan,
        splits,
    })
}

fn check_len(len: u8, max: u8, net: IpNet) -> Result<(), String> {
    if len > max {
        return Err(format!(
            "/{len} is not a valid length for {} ({net} tops out at /{max})",
            carve::family(&net)
        ));
    }
    Ok(())
}

/// Which subnet of the split `sub` is, counting from zero.
fn index_of(parent: IpNet, sub: IpNet) -> u128 {
    let shift = u32::from(sub.max_prefix_len() - sub.prefix_len());
    (to_u128(sub.network()) - to_u128(parent.network())) >> shift
}

pub fn to_u128(addr: IpAddr) -> u128 {
    match addr {
        IpAddr::V4(a) => u128::from(u32::from(a)),
        IpAddr::V6(a) => u128::from(a),
    }
}

pub fn first_subnet(block: IpNet, len: u8) -> IpNet {
    IpNet::new(block.network(), len)
        .expect("len is a valid length for the family")
        .trunc()
}

pub fn last_subnet(block: IpNet, len: u8) -> IpNet {
    IpNet::new(block.broadcast(), len)
        .expect("len is a valid length for the family")
        .trunc()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops;

    fn report(prefix: &str, ops_str: &[&str]) -> Report {
        let parsed: Vec<Op> = ops_str.iter().map(|o| ops::parse(o).unwrap()).collect();
        build(prefix, prefix.parse().unwrap(), &parsed).unwrap()
    }

    fn err(prefix: &str, op: &str) -> String {
        let parsed = vec![ops::parse(op).unwrap()];
        match build(prefix, prefix.parse().unwrap(), &parsed) {
            Err(e) => e,
            Ok(_) => panic!("expected {op} to be rejected"),
        }
    }

    #[test]
    fn split_of_a_whole_prefix() {
        let r = report("2001:db8::/52", &["/64"]);
        let s = &r.splits[0];
        assert_eq!(s.counts()[0].as_u128(), Some(4096));
        assert_eq!(s.first().unwrap().to_string(), "2001:db8::/64");
        assert_eq!(s.last().unwrap().to_string(), "2001:db8:0:fff::/64");
        assert_eq!(s.subnets().take(2).count(), 2);
    }

    #[test]
    fn splitting_the_full_ipv6_space_does_not_hang() {
        let r = report("::/0", &["/128"]);
        let s = &r.splits[0];
        assert_eq!(s.counts()[0].as_u128(), None);
        assert_eq!(s.subnets().take(3).count(), 3);
    }

    #[test]
    fn a_split_after_a_carve_describes_the_remainder() {
        let r = report("2001:db8::/52", &["-56", "/64"]);
        let s = &r.splits[0];
        assert_eq!(s.source, Source::Remainder);
        // 4096 /64s less the 256 inside the carved /56.
        let total: u128 = s.counts().iter().map(|c| c.as_u128().unwrap()).sum();
        assert_eq!(total, 4096 - 256);
    }

    #[test]
    fn supernets_report_how_many_siblings_they_hold() {
        let r = report("10.1.2.0/24", &["+16"]);
        assert_eq!(r.supernets[0].net.to_string(), "10.1.0.0/16");
        assert_eq!(r.supernets[0].siblings.as_u128(), Some(256));
    }

    #[test]
    fn lookups_locate_an_address_within_a_split() {
        let r = report("2001:db8::/52", &["/64", "=2001:db8:0:3::5"]);
        let l = &r.lookups[0];
        assert!(l.inside);
        assert_eq!(l.positions[0].1.to_string(), "2001:db8:0:3::/64");
        assert_eq!(l.positions[0].2, 3);
    }

    #[test]
    fn lookups_outside_the_prefix_say_so() {
        let r = report("10.0.0.0/24", &["=10.0.9.1"]);
        assert!(!r.lookups[0].inside);
        assert!(r.lookups[0].positions.is_empty());
    }

    #[test]
    fn duplicate_split_lengths_collapse() {
        assert_eq!(report("10.0.0.0/8", &["/24", "/24"]).splits.len(), 1);
    }

    #[test]
    fn nonsensical_lengths_are_rejected_with_advice() {
        assert!(err("10.0.0.0/24", "/16").contains("+16"));
        assert!(err("10.0.0.0/24", "+30").contains("/30"));
        assert!(err("10.0.0.0/24", "/64").contains("IPv4"));
        assert!(err("10.0.0.0/24", "=2001:db8::1").contains("IPv6"));
        assert!(err("10.0.0.0/8", "-24*70000").contains("65536"));
    }

    #[test]
    fn a_split_skips_remainder_blocks_that_are_too_small() {
        // Carving a /25 leaves a /25; a /24 split of the remainder has nothing
        // big enough to divide.
        let r = report("10.0.0.0/24", &["-25", "/24"]);
        assert_eq!(r.splits[0].blocks.len(), 0);
        assert_eq!(r.splits[0].too_small, 1);
    }
}
