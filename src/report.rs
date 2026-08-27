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
    pub aggregates: Vec<Aggregation>,
    pub neighbours: Vec<Neighbour>,
    pub lookups: Vec<Lookup>,
    pub picks: Vec<Pick>,
    pub carve: Option<Plan>,
    pub splits: Vec<Split>,
}

/// `+<prefix>` - the smallest prefix holding both.
pub struct Aggregation {
    pub with: IpNet,
    pub net: IpNet,
    /// The aggregate is exactly the two inputs and nothing else.
    pub exact: bool,
    /// One of the two already contains the other.
    pub nested: bool,
    /// Space inside the aggregate that neither input covers.
    pub spare: Vec<IpNet>,
}

/// `^N` - a prefix of the same size, N blocks along.
pub struct Neighbour {
    pub step: i64,
    pub net: IpNet,
}

/// `@N` - the Nth subnet of a requested split.
pub struct Pick {
    /// As the user wrote it, so `-1` stays `-1` in the output.
    pub index: i64,
    pub len: u8,
    /// The index actually used, after resolving a negative one.
    pub resolved: u128,
    pub net: IpNet,
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
    let mut aggregates = Vec::new();
    let mut neighbours = Vec::new();
    let mut lookups = Vec::new();
    let mut pick_indexes = Vec::new();

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
            Op::Aggregate(other) => {
                if other.addr().is_ipv4() != net.addr().is_ipv4() {
                    return Err(format!(
                        "{other} is {} but {net} is {}",
                        if other.addr().is_ipv4() {
                            "IPv4"
                        } else {
                            "IPv6"
                        },
                        carve::family(&net)
                    ));
                }
                aggregates.push(aggregate(net, other.trunc()));
            }
            Op::Step(n) => neighbours.push(Neighbour {
                step: *n,
                net: step(net, *n)?,
            }),
            Op::Nth(n) => pick_indexes.push(*n),
            Op::Carve { len, count } => {
                if *count > carve::MAX_REQUEST_COUNT {
                    return Err(format!(
                        "{count} subnets is more than this carves in one go \
                         (limit {}); use /{len} to describe a split that size instead",
                        carve::MAX_REQUEST_COUNT
                    ));
                }
                // Expanded so each subnet gets its own outcome, and its own
                // line in the output.
                for _ in 0..*count {
                    requests.push(Request::Floating(*len));
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

    let mut picks = Vec::new();
    for index in &pick_indexes {
        if split_lens.is_empty() {
            return Err(format!(
                "@{index} needs a split length to count within, for example /64 @{index}"
            ));
        }
        for len in &split_lens {
            let (resolved, sub) = nth(net, *len, *index)?;
            picks.push(Pick {
                index: *index,
                len: *len,
                resolved,
                net: sub,
            });
        }
    }

    Ok(Report {
        info,
        supernets,
        aggregates,
        neighbours,
        lookups,
        picks,
        carve: plan,
        splits,
    })
}

/// The smallest prefix holding both, plus whatever space that leaves over.
fn aggregate(a: IpNet, b: IpNet) -> Aggregation {
    let bits = a.max_prefix_len();
    let diff = to_u128(a.network()) ^ to_u128(b.network());
    let common = if diff == 0 {
        bits
    } else {
        // leading_zeros counts across the full u128, so drop the padding an
        // IPv4 address sits behind.
        (diff.leading_zeros() as u8).saturating_sub(128 - bits)
    };
    let len = common.min(a.prefix_len()).min(b.prefix_len());
    let net = IpNet::new(a.addr(), len)
        .expect("a length no longer than either input")
        .trunc();

    let nested = a.contains(&b) || b.contains(&a);
    // Two disjoint prefixes fill their aggregate exactly only when they are
    // siblings; anything else leaves a gap.
    let exact = !nested && a.prefix_len() == b.prefix_len() && a.is_sibling(&b);
    let spare = if nested || exact {
        Vec::new()
    } else {
        // Reuse the allocator: reserve both inputs inside the aggregate and
        // whatever stays free is the space neither of them covers.
        carve::plan(net, &[carve::Request::Fixed(a), carve::Request::Fixed(b)]).free
    };

    Aggregation {
        with: b,
        net,
        exact,
        nested,
        spare,
    }
}

/// The prefix `n` blocks of the same size away.
fn step(net: IpNet, n: i64) -> Result<IpNet, String> {
    let bits = net.max_prefix_len();
    let host = u32::from(bits - net.prefix_len());
    if host >= 128 {
        // The whole address space is one block; there is nowhere to step to.
        return if n == 0 {
            Ok(net)
        } else {
            Err(format!(
                "{net} is the whole address space, so ^{n} runs off it"
            ))
        };
    }
    let block = 1u128 << host;
    let last = if bits == 32 {
        u128::from(u32::MAX)
    } else {
        u128::MAX
    };
    let start = to_u128(net.network());

    // Done unsigned in both directions: n * block can be 2^127, which does
    // not fit in an i128.
    let offset = (n.unsigned_abs() as u128)
        .checked_mul(block)
        .ok_or_else(|| out_of_range(net, n))?;
    let moved = if n >= 0 {
        start.checked_add(offset)
    } else {
        start.checked_sub(offset)
    }
    .ok_or_else(|| out_of_range(net, n))?;

    if moved > last - block + 1 {
        return Err(out_of_range(net, n));
    }
    Ok(IpNet::new(from_u128(moved, bits == 32), net.prefix_len())
        .expect("length unchanged")
        .trunc())
}

fn out_of_range(net: IpNet, n: i64) -> String {
    format!("^{n} runs off the end of the address space from {net}")
}

/// The Nth subnet of `len` inside `parent`, resolving a negative index from
/// the end. Returns the resolved index alongside the subnet.
fn nth(parent: IpNet, len: u8, n: i64) -> Result<(u128, IpNet), String> {
    if len > parent.max_prefix_len() || len < parent.prefix_len() {
        return Err(format!("/{len} is not a subnet length inside {parent}"));
    }
    let exp = u32::from(len - parent.prefix_len());
    let too_far = || {
        format!(
            "@{n} is outside the {} subnets of /{len} in {parent}",
            crate::num::Count::pow2(exp).short()
        )
    };

    let index = if n >= 0 {
        let i = n as u128;
        if exp < 128 && i >= (1u128 << exp) {
            return Err(too_far());
        }
        i
    } else {
        let back = u128::from(n.unsigned_abs());
        if exp >= 128 {
            // count is 2^128, which does not fit; count back from the top.
            u128::MAX.checked_sub(back - 1).ok_or_else(too_far)?
        } else {
            (1u128 << exp).checked_sub(back).ok_or_else(too_far)?
        }
    };

    let host = u32::from(parent.max_prefix_len() - len);
    let offset = index << host;
    let addr = to_u128(parent.network()) + offset;
    Ok((
        index,
        IpNet::new(from_u128(addr, parent.addr().is_ipv4()), len)
            .expect("length already checked")
            .trunc(),
    ))
}

fn from_u128(v: u128, ipv4: bool) -> IpAddr {
    if ipv4 {
        IpAddr::V4(std::net::Ipv4Addr::from(v as u32))
    } else {
        IpAddr::V6(std::net::Ipv6Addr::from(v))
    }
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
        errs(prefix, &[op])
    }

    fn errs(prefix: &str, ops_str: &[&str]) -> String {
        let parsed: Vec<Op> = ops_str.iter().map(|o| ops::parse(o).unwrap()).collect();
        match build(prefix, prefix.parse().unwrap(), &parsed) {
            Err(e) => e,
            Ok(_) => panic!("expected {ops_str:?} to be rejected"),
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
    fn aggregating_siblings_is_exact() {
        let r = report("10.0.0.0/24", &["+10.0.1.0/24"]);
        let a = &r.aggregates[0];
        assert_eq!(a.net.to_string(), "10.0.0.0/23");
        assert!(a.exact);
        assert!(!a.nested);
        assert!(a.spare.is_empty());
    }

    #[test]
    fn aggregating_non_siblings_reports_the_gap() {
        let r = report("10.0.0.0/24", &["+10.0.3.0/24"]);
        let a = &r.aggregates[0];
        assert_eq!(a.net.to_string(), "10.0.0.0/22");
        assert!(!a.exact);
        assert_eq!(
            a.spare.iter().map(|n| n.to_string()).collect::<Vec<_>>(),
            vec!["10.0.1.0/24", "10.0.2.0/24"]
        );
    }

    #[test]
    fn aggregating_a_nested_prefix_is_the_outer_one() {
        let r = report("10.0.0.0/16", &["+10.0.5.0/24"]);
        let a = &r.aggregates[0];
        assert_eq!(a.net.to_string(), "10.0.0.0/16");
        assert!(a.nested);
        assert!(!a.exact);
        assert!(a.spare.is_empty());

        // The same holds when the smaller prefix is the one on the left.
        let r = report("10.0.5.0/24", &["+10.0.0.0/16"]);
        assert_eq!(r.aggregates[0].net.to_string(), "10.0.0.0/16");
        assert!(r.aggregates[0].nested);
    }

    #[test]
    fn aggregating_ipv6_uses_the_right_bit_width() {
        // The IPv4 padding correction must not leak into IPv6.
        let r = report("2001:db8::/48", &["+2001:db8:1::/48"]);
        assert_eq!(r.aggregates[0].net.to_string(), "2001:db8::/47");
        assert!(r.aggregates[0].exact);

        let r = report("2001:db8::/32", &["+2001:dba::/32"]);
        assert_eq!(r.aggregates[0].net.to_string(), "2001:db8::/30");
    }

    #[test]
    fn stepping_walks_blocks_of_the_same_size() {
        let r = report("10.0.4.0/22", &["^1", "^2", "^-1", "^0"]);
        let got: Vec<String> = r.neighbours.iter().map(|n| n.net.to_string()).collect();
        assert_eq!(
            got,
            vec!["10.0.8.0/22", "10.0.12.0/22", "10.0.0.0/22", "10.0.4.0/22"]
        );
    }

    #[test]
    fn stepping_off_the_address_space_is_an_error() {
        assert!(err("255.255.252.0/22", "^1").contains("runs off"));
        assert!(err("0.0.0.0/22", "^-1").contains("runs off"));
        assert!(err("::/0", "^1").contains("whole address space"));
    }

    #[test]
    fn stepping_a_half_of_ipv6_does_not_overflow() {
        // The block size here is 2^127, which does not fit in an i128.
        let r = report("::/1", &["^1"]);
        assert_eq!(r.neighbours[0].net.to_string(), "8000::/1");
        let r = report("8000::/1", &["^-1"]);
        assert_eq!(r.neighbours[0].net.to_string(), "::/1");
    }

    #[test]
    fn picking_counts_from_either_end() {
        let r = report("2001:db8::/52", &["/64", "@0", "@3", "@-1"]);
        let got: Vec<String> = r.picks.iter().map(|p| p.net.to_string()).collect();
        assert_eq!(
            got,
            vec!["2001:db8::/64", "2001:db8:0:3::/64", "2001:db8:0:fff::/64"]
        );
        assert_eq!(r.picks[2].resolved, 4095);
        assert_eq!(r.picks[2].index, -1);
    }

    #[test]
    fn picking_is_the_inverse_of_a_lookup() {
        let r = report("2001:db8::/52", &["/64", "@3", "=2001:db8:0:3::5"]);
        assert_eq!(r.picks[0].net, r.lookups[0].positions[0].1);
        assert_eq!(r.picks[0].resolved, r.lookups[0].positions[0].2);
    }

    #[test]
    fn picking_the_last_of_the_whole_ipv6_space() {
        // count is 2^128, which does not fit in a u128 at all.
        let r = report("::/0", &["/128", "@-1"]);
        assert_eq!(
            r.picks[0].net.to_string(),
            "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128"
        );
        assert_eq!(r.picks[0].resolved, u128::MAX);
    }

    #[test]
    fn picking_applies_to_every_requested_split() {
        let r = report("10.0.0.0/16", &["/24", "/20", "@1"]);
        let got: Vec<String> = r.picks.iter().map(|p| p.net.to_string()).collect();
        assert_eq!(got, vec!["10.0.1.0/24", "10.0.16.0/20"]);
    }

    #[test]
    fn picking_out_of_range_or_without_a_split_is_rejected() {
        assert!(err("2001:db8::/52", "@3").contains("needs a split length"));
        let e = errs("2001:db8::/52", &["/64", "@4096"]);
        assert!(e.contains("outside the 4,096 subnets"), "{e}");
        let e = errs("2001:db8::/52", &["/64", "@-4097"]);
        assert!(e.contains("outside the 4,096 subnets"), "{e}");
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
