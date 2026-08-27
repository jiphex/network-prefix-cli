//! Turns a prefix plus a list of operators into everything we want to print.
//!
//! Operators are grouped rather than applied strictly left to right: all the
//! carve operators feed one allocator run, and any split then describes what
//! that run left behind. Splitting the remainder is what a planner actually
//! wants to know - "after those allocations, how many /64s have I still got?"

use crate::carve::{self, Direction, Plan, Request};
use crate::info::Info;
use crate::num::Count;
use crate::ops::{Op, Target};
use crate::zones;
use ipnet::IpNet;
use std::collections::BinaryHeap;
use std::net::IpAddr;

pub struct Report {
    pub info: Info,
    pub zones: Vec<zones::Zones>,
    pub parts: Vec<Parts>,
    pub shares: Vec<Shares>,
    pub supernets: Vec<Supernet>,
    pub aggregates: Vec<Aggregation>,
    pub neighbours: Vec<Neighbour>,
    pub lookups: Vec<Lookup>,
    pub picks: Vec<Pick>,
    pub carve: Option<Plan>,
    pub splits: Vec<Split>,
}

/// `+<prefix>` - the smallest prefix holding the prefix under inspection and
/// every prefix named alongside it.
pub struct Aggregation {
    /// The prefixes named with `+`, in the order they were given.
    pub with: Vec<IpNet>,
    pub net: IpNet,
    /// The inputs fill the aggregate between them, with nothing left over.
    pub exact: bool,
    /// One input already contains another.
    pub nested: bool,
    /// Space inside the aggregate that no input covers.
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

/// `%M` - the space divided into exactly M subnets.
pub struct Parts {
    pub wanted: u64,
    pub source: Source,
    /// Exactly `wanted` blocks, in address order.
    pub blocks: Vec<IpNet>,
}

impl Parts {
    /// The lengths used, longest first, with how many of each. At most two
    /// distinct lengths ever appear.
    pub fn sizes(&self) -> Vec<(u8, usize)> {
        let mut sizes: Vec<(u8, usize)> = Vec::new();
        for block in &self.blocks {
            match sizes.iter_mut().find(|(len, _)| *len == block.prefix_len()) {
                Some((_, n)) => *n += 1,
                None => sizes.push((block.prefix_len(), 1)),
            }
        }
        sizes.sort_by_key(|(len, _)| *len);
        sizes
    }
}

/// `%a:b:c` - the space shared out in the ratio asked for.
pub struct Shares {
    /// The ratio as it was written.
    pub wanted: Vec<u64>,
    pub source: Source,
    /// The blocks each share was given, in the order the shares were written.
    /// Together they tile the space exactly.
    pub granted: Vec<Vec<IpNet>>,
    /// What each share actually got, in units of the granularity used.
    /// A ragged remainder can force a very fine unit, so these are not
    /// small numbers in general.
    pub units: Vec<u128>,
    /// True when the ratio came out exactly as asked.
    pub exact: bool,
}

/// Past this a ratio has stopped being something anyone reads, and the
/// percentages say more.
const READABLE_SHARE: u128 = 9_999;

impl Shares {
    /// The ratio actually achieved, in its lowest terms. Equal to `wanted`
    /// reduced whenever the split was exact.
    pub fn achieved(&self) -> Vec<u128> {
        let g = self.units.iter().copied().fold(0, gcd).max(1);
        self.units.iter().map(|u| u / g).collect()
    }

    /// The achieved ratio, or `None` when it has too many digits to be worth
    /// reading. Sharing out a space that is already in ragged pieces can
    /// force a very fine unit, and `1431655765:715827882:715827883` tells a
    /// reader nothing that "roughly 2:1:1" did not.
    pub fn readable_ratio(&self) -> Option<Vec<u128>> {
        let got = self.achieved();
        got.iter().all(|g| *g <= READABLE_SHARE).then_some(got)
    }

    /// True when the ratio could be cut exactly out of a single block - its
    /// parts, reduced, add up to a power of two.
    ///
    /// This is what separates the two ways a share can come out inexact: a
    /// ratio like 2:1 that no prefix can express, and a ratio like 2:1:1 that
    /// any prefix can, applied to a remainder that is no longer one block.
    pub fn ratio_is_dyadic(&self) -> bool {
        let g = self
            .wanted
            .iter()
            .copied()
            .fold(0u64, |a, b| gcd(u128::from(a), u128::from(b)) as u64);
        let total: u64 = self.wanted.iter().sum::<u64>() / g.max(1);
        total.is_power_of_two()
    }

    /// Each share as a percentage of the space, for when the ratio is not.
    pub fn percentages(&self) -> Vec<f64> {
        let total: u128 = self.units.iter().sum();
        self.units
            .iter()
            .map(|u| *u as f64 * 100.0 / total as f64)
            .collect()
    }

    /// Addresses in one share, as a sum over the blocks it was given.
    pub fn counts(&self, i: usize) -> Vec<Count> {
        self.granted[i]
            .iter()
            .map(|n| Count::pow2(u32::from(n.max_prefix_len() - n.prefix_len())))
            .collect()
    }
}

fn gcd(a: u128, b: u128) -> u128 {
    if b == 0 { a } else { gcd(b, a % b) }
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

pub fn build(input: &str, net: IpNet, ops: &[Op], direction: Direction) -> Result<Report, String> {
    let info = Info::new(input, net);
    let net = info.net;
    let max = net.max_prefix_len();

    let mut requests = Vec::new();
    let mut split_lens = Vec::new();
    let mut part_counts: Vec<u64> = Vec::new();
    let mut share_ratios: Vec<Vec<u64>> = Vec::new();
    let mut zone_sets = Vec::new();
    let mut supernets = Vec::new();
    let mut aggregate_with: Vec<IpNet> = Vec::new();
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
            Op::Parts(m) => {
                if *m > carve::MAX_REQUEST_COUNT {
                    return Err(format!(
                        "{m} subnets is more than this splits in one go (limit {}); \
                         use /N for a split that size",
                        crate::num::group(&carve::MAX_REQUEST_COUNT.to_string())
                    ));
                }
                if !part_counts.contains(m) {
                    part_counts.push(*m);
                }
            }
            Op::Shares(ratio) => {
                let total: u64 = ratio
                    .iter()
                    .try_fold(0u64, |a, b| a.checked_add(*b))
                    .ok_or("those shares add up to more than this can count, let alone divide")?;
                if total > carve::MAX_REQUEST_COUNT {
                    return Err(format!(
                        "those shares add up to {}, which is more than this divides in one go \
                         (limit {})",
                        crate::num::group(&total.to_string()),
                        crate::num::group(&carve::MAX_REQUEST_COUNT.to_string())
                    ));
                }
                if !share_ratios.contains(ratio) {
                    share_ratios.push(ratio.clone());
                }
            }
            Op::Zones(boundary) => zone_sets.push(zones::zones(&net, *boundary)?),
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
                // Collected rather than aggregated one at a time: several
                // `+` operators describe one aggregate covering all of them,
                // not a series of pairings with the prefix under inspection.
                aggregate_with.push(other.trunc());
            }
            Op::Step(n) => neighbours.push(Neighbour {
                step: *n,
                net: step(net, *n)?,
            }),
            Op::Nth(n) => pick_indexes.push(*n),
            Op::Carve { len, count, label } => {
                if *count > carve::MAX_REQUEST_COUNT {
                    return Err(format!(
                        "{count} subnets is more than this carves in one go \
                         (limit {}); use /{len} to describe a split that size instead",
                        crate::num::group(&carve::MAX_REQUEST_COUNT.to_string())
                    ));
                }
                // Expanded so each subnet gets its own outcome, and its own
                // line in the output.
                for _ in 0..*count {
                    requests.push(Request::floating(*len).named(label.clone()));
                }
            }
            Op::Exclude { net: target, label } => {
                requests.push(Request::fixed(*target).named(label.clone()))
            }
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

    let aggregates = if aggregate_with.is_empty() {
        Vec::new()
    } else {
        vec![aggregate(net, &aggregate_with)]
    };

    let plan = (!requests.is_empty()).then(|| carve::plan(net, &requests, direction));

    let parts = part_counts
        .iter()
        .map(|m| {
            let (source, blocks) = match &plan {
                Some(plan) => (Source::Remainder, plan.free.clone()),
                None => (Source::Whole, vec![net]),
            };
            divide(&blocks, *m).map(|blocks| Parts {
                wanted: *m,
                source,
                blocks,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let shares = share_ratios
        .iter()
        .map(|ratio| {
            let (source, blocks) = match &plan {
                Some(plan) => (Source::Remainder, plan.free.clone()),
                None => (Source::Whole, vec![net]),
            };
            share(&blocks, ratio).map(|(granted, units, exact)| Shares {
                wanted: ratio.clone(),
                source,
                granted,
                units,
                exact,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

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
        zones: zone_sets,
        parts,
        shares,
        supernets,
        aggregates,
        neighbours,
        lookups,
        picks,
        carve: plan,
        splits,
    })
}

/// Divide `blocks` into exactly `m` subnets that tile the same space.
///
/// Repeatedly halving the largest block is what makes the result as even as
/// the space allows: every step narrows the gap between the biggest and the
/// smallest, so at most two lengths are ever in play. When `m` is a power of
/// two the result is the uniform split `/N` would have given.
fn divide(blocks: &[IpNet], m: u64) -> Result<Vec<IpNet>, String> {
    let m = usize::try_from(m).map_err(|_| format!("{m} subnets is more than this can hold"))?;
    if m < blocks.len() {
        return Err(format!(
            "the space is already {} separate block{}, so it cannot become {m}",
            blocks.len(),
            if blocks.len() == 1 { "" } else { "s" }
        ));
    }

    let mut heap: BinaryHeap<BySize> = blocks.iter().copied().map(BySize).collect();
    while heap.len() < m {
        let BySize(largest) = heap
            .pop()
            .expect("m is at least one and the heap is non-empty");
        if largest.prefix_len() == largest.max_prefix_len() {
            // Everything left is a single address, so there is nothing to
            // halve and the count cannot go any higher.
            return Err(format!(
                "{} cannot be divided into {m} subnets; {} is the most it holds",
                blocks
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                heap.len() + 1
            ));
        }
        let (low, high) = carve::halves(&largest);
        heap.push(BySize(low));
        heap.push(BySize(high));
    }

    let mut out: Vec<IpNet> = heap.into_iter().map(|BySize(n)| n).collect();
    out.sort();
    Ok(out)
}

/// Share `blocks` out in the ratio `wanted`.
///
/// The space is cut into equal units small enough that every block is a whole
/// number of them and there are at least as many units as there are shares.
/// The units are then apportioned by largest remainder and each share's run
/// is glued back into the fewest aligned blocks that cover it.
///
/// Working in units rather than in addresses is what keeps the result exact:
/// every unit goes to exactly one share, so the shares tile the space however
/// ragged it started out, and a share whose run does not fit one aligned block
/// gets several rather than being rounded.
///
/// A ratio is only exactly representable when its parts, reduced, sum to a
/// power of two - `2:1:1` can be cut from a prefix but `2:1` cannot, because
/// two thirds of a prefix is not a prefix. Anything else lands on the nearest
/// the units allow, which is the same bargain `%M` already makes.
type Shared = (Vec<Vec<IpNet>>, Vec<u128>, bool);

fn share(blocks: &[IpNet], wanted: &[u64]) -> Result<Shared, String> {
    let total: u128 = wanted.iter().map(|w| u128::from(*w)).sum();
    let ratio = || {
        wanted
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(":")
    };
    let space = || {
        blocks
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };

    // Units start at the size of the smallest block, which is the coarsest
    // granularity that still divides every block evenly.
    let host = |n: &IpNet| u32::from(n.max_prefix_len() - n.prefix_len());
    let mut unit_bits = blocks.iter().map(host).min().unwrap_or(0);
    let mut units: u128 = blocks
        .iter()
        .map(|b| 1u128 << (host(b) - unit_bits))
        .sum::<u128>();

    // Halving the unit until there are enough to go round. Only the whole-
    // prefix case ever gets here - it starts at a single unit - and it stops
    // the moment it has enough, so `units` stays under the ratio's own limit
    // and cannot overflow on the way.
    while units < total {
        if unit_bits == 0 {
            return Err(format!(
                "{} cannot be shared {}; it holds {} address{} and the ratio needs {}",
                space(),
                ratio(),
                crate::num::describe_sum(
                    &blocks
                        .iter()
                        .map(|b| Count::pow2(host(b)))
                        .collect::<Vec<_>>()
                ),
                if units == 1 { "" } else { "es" },
                crate::num::group(&total.to_string()),
            ));
        }
        unit_bits -= 1;
        units *= 2;
    }

    // Largest remainder: floor everyone first, then hand the leftover units
    // to whoever was rounded down hardest. Ties go to the earlier share, so
    // the result depends only on the ratio and not on the sort.
    let mut got: Vec<u128> = wanted
        .iter()
        .map(|w| u128::from(*w) * units / total)
        .collect();
    let mut spare = units - got.iter().sum::<u128>();
    let mut order: Vec<usize> = (0..wanted.len()).collect();
    order.sort_by_key(|&i| {
        let remainder = u128::from(wanted[i]) * units % total;
        (std::cmp::Reverse(remainder), i)
    });
    for &i in &order {
        if spare == 0 {
            break;
        }
        got[i] += 1;
        spare -= 1;
    }

    let exact = wanted
        .iter()
        .all(|w| (u128::from(*w) * units).is_multiple_of(total));

    // Walk the blocks in address order, cutting each share's run off the
    // front. A run that spans a block boundary simply continues into the
    // next block, which is what lets a ragged remainder be shared at all.
    let mut granted: Vec<Vec<IpNet>> = vec![Vec::new(); wanted.len()];
    let mut blocks = blocks.iter();
    let mut current = blocks.next().copied();
    let mut left_in_block: u128 = current.map_or(0, |b| 1u128 << (host(&b) - unit_bits));
    let mut offset: u128 = 0;
    for (i, mut owed) in got.iter().copied().enumerate() {
        while owed > 0 {
            let block = current.expect("the units were counted from these blocks");
            let take = owed.min(left_in_block);
            cut(block, unit_bits, offset, take, &mut granted[i]);
            owed -= take;
            offset += take;
            left_in_block -= take;
            if left_in_block == 0 {
                current = blocks.next().copied();
                left_in_block = current.map_or(0, |b| 1u128 << (host(&b) - unit_bits));
                offset = 0;
            }
        }
    }

    Ok((granted, got, exact))
}

/// The fewest aligned blocks covering `len` units starting `start` units into
/// `container`. Each step takes the largest block the offset's alignment and
/// the units remaining will both allow, which is the minimal cover.
fn cut(container: IpNet, unit_bits: u32, mut start: u128, mut len: u128, out: &mut Vec<IpNet>) {
    let base = to_u128(container.network());
    let ipv4 = container.addr().is_ipv4();
    while len > 0 {
        let by_align = if start == 0 {
            u128::MAX
        } else {
            1u128 << start.trailing_zeros()
        };
        let by_len = 1u128 << (127 - len.leading_zeros());
        let size = by_align.min(by_len);
        let host = unit_bits + size.trailing_zeros();
        out.push(
            IpNet::new(
                from_u128(base + (start << unit_bits), ipv4),
                container.max_prefix_len() - host as u8,
            )
            .expect("a length between the container's and a host route")
            .trunc(),
        );
        start += size;
        len -= size;
    }
}

/// Orders blocks largest first, and among equals by lowest address, so the
/// division is deterministic rather than however the heap happened to settle.
#[derive(PartialEq, Eq)]
struct BySize(IpNet);

impl Ord for BySize {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let host = |n: &IpNet| n.max_prefix_len() - n.prefix_len();
        host(&self.0)
            .cmp(&host(&other.0))
            .then_with(|| to_u128(other.0.network()).cmp(&to_u128(self.0.network())))
    }
}

impl PartialOrd for BySize {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The smallest prefix holding every input, plus whatever space that leaves
/// over.
fn aggregate(base: IpNet, with: &[IpNet]) -> Aggregation {
    let inputs: Vec<IpNet> = std::iter::once(base).chain(with.iter().copied()).collect();
    let bits = base.max_prefix_len();

    // The aggregate reaches back to the longest run of leading bits that every
    // input shares, and can be no longer than the shortest input.
    let mut len = inputs.iter().map(IpNet::prefix_len).min().unwrap_or(bits);
    for other in &inputs {
        let diff = to_u128(base.network()) ^ to_u128(other.network());
        let common = if diff == 0 {
            bits
        } else {
            // leading_zeros counts across the full u128, so drop the padding
            // an IPv4 address sits behind.
            (diff.leading_zeros() as u8).saturating_sub(128 - bits)
        };
        len = len.min(common);
    }
    let net = IpNet::new(base.addr(), len)
        .expect("a length no longer than any input")
        .trunc();

    // Aligned prefixes either nest or are disjoint, so dropping every input
    // that another one already contains leaves a disjoint set - which is what
    // the allocator needs, and what the union of the inputs actually is.
    let mut maximal: Vec<IpNet> = Vec::new();
    for (i, candidate) in inputs.iter().enumerate() {
        let covered = inputs
            .iter()
            .enumerate()
            .any(|(j, other)| other.contains(candidate) && (other != candidate || j < i));
        if !covered {
            maximal.push(*candidate);
        }
    }
    let nested = maximal.len() < inputs.len();

    // Reserve the union inside the aggregate; whatever stays free is the space
    // no input covers.
    let requests: Vec<carve::Request> = maximal.into_iter().map(carve::Request::fixed).collect();
    // Direction only steers floating requests, and these are all fixed.
    let spare = carve::plan(net, &requests, Direction::default()).free;

    Aggregation {
        with: with.to_vec(),
        net,
        exact: spare.is_empty(),
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
        build(prefix, prefix.parse().unwrap(), &parsed, Direction::Bottom).unwrap()
    }

    fn err(prefix: &str, op: &str) -> String {
        errs(prefix, &[op])
    }

    fn errs(prefix: &str, ops_str: &[&str]) -> String {
        let parsed: Vec<Op> = ops_str.iter().map(|o| ops::parse(o).unwrap()).collect();
        match build(prefix, prefix.parse().unwrap(), &parsed, Direction::Bottom) {
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
        // The outer prefix is the aggregate, so the inputs do fill it.
        assert!(a.exact);
        assert!(a.spare.is_empty());

        // The same holds when the smaller prefix is the one on the left.
        let r = report("10.0.5.0/24", &["+10.0.0.0/16"]);
        assert_eq!(r.aggregates[0].net.to_string(), "10.0.0.0/16");
        assert!(r.aggregates[0].nested);
    }

    #[test]
    fn several_pluses_make_one_aggregate_not_several_pairs() {
        let r = report("10.0.0.0/24", &["+10.0.1.0/24", "+10.1.0.0/16"]);
        assert_eq!(r.aggregates.len(), 1, "one aggregate, not a pairing each");

        let a = &r.aggregates[0];
        assert_eq!(
            a.with.iter().map(|n| n.to_string()).collect::<Vec<_>>(),
            vec!["10.0.1.0/24", "10.1.0.0/16"]
        );
        assert_eq!(a.net.to_string(), "10.0.0.0/15");

        // 10.0.1.0/24 was named by the user, so it is not spare space.
        let spare: Vec<String> = a.spare.iter().map(|n| n.to_string()).collect();
        assert!(
            !spare.contains(&"10.0.1.0/24".to_string()),
            "a named prefix was counted as unused: {spare:?}"
        );
        assert_eq!(
            spare,
            vec![
                "10.0.2.0/23",
                "10.0.4.0/22",
                "10.0.8.0/21",
                "10.0.16.0/20",
                "10.0.32.0/19",
                "10.0.64.0/18",
                "10.0.128.0/17",
            ]
        );
    }

    #[test]
    fn the_aggregate_and_its_spare_account_for_every_input() {
        // Whatever the inputs, the aggregate contains all of them and the
        // spare blocks never overlap any of them.
        for (base, ops) in [
            ("10.0.0.0/24", vec!["+10.0.1.0/24", "+10.1.0.0/16"]),
            ("10.0.0.0/24", vec!["+10.0.1.0/24", "+10.0.2.0/23"]),
            ("10.0.0.0/24", vec!["+10.0.0.128/25", "+10.0.1.0/24"]),
            ("10.0.0.0/16", vec!["+10.0.5.0/24"]),
            (
                "192.168.4.0/24",
                vec!["+192.168.9.0/24", "+192.168.200.0/22"],
            ),
            (
                "2001:db8::/48",
                vec!["+2001:db8:1::/48", "+2001:db8:9::/44"],
            ),
        ] {
            let r = report(base, &ops);
            let a = &r.aggregates[0];
            let inputs: Vec<IpNet> = std::iter::once(base.parse().unwrap())
                .chain(a.with.iter().copied())
                .collect();

            for input in &inputs {
                assert!(a.net.contains(input), "{} missing {input}", a.net);
                for spare in &a.spare {
                    assert!(
                        !spare.contains(input) && !input.contains(spare),
                        "spare {spare} overlaps input {input}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_single_plus_is_unchanged() {
        let r = report("10.0.0.0/24", &["+10.0.1.0/24"]);
        assert_eq!(r.aggregates.len(), 1);
        assert_eq!(r.aggregates[0].with.len(), 1);
        assert_eq!(r.aggregates[0].net.to_string(), "10.0.0.0/23");
        assert!(r.aggregates[0].exact);
        assert!(!r.aggregates[0].nested);
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
    fn dividing_into_a_count_tiles_the_prefix_exactly() {
        // Whatever the count, the pieces cover the parent once over.
        for m in 1..=64u64 {
            let r = report("10.0.0.0/24", &[&format!("%{m}")]);
            let blocks = &r.parts[0].blocks;
            assert_eq!(blocks.len(), m as usize, "%{m} produced the wrong count");

            let addresses: u128 = blocks
                .iter()
                .map(|n| 1u128 << (n.max_prefix_len() - n.prefix_len()))
                .sum();
            assert_eq!(addresses, 256, "%{m} does not fill the /24");

            for pair in blocks.windows(2) {
                assert_eq!(
                    to_u128(pair[0].broadcast()) + 1,
                    to_u128(pair[1].network()),
                    "%{m}: {} and {} do not abut",
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    #[test]
    fn dividing_uses_at_most_two_lengths_one_bit_apart() {
        // "As even as the space allows" means exactly this.
        for m in 1..=64u64 {
            let r = report("2001:db8::/56", &[&format!("%{m}")]);
            let sizes = r.parts[0].sizes();
            assert!(sizes.len() <= 2, "%{m} used {} lengths", sizes.len());
            if let [(short, _), (long, _)] = sizes[..] {
                assert_eq!(long, short + 1, "%{m} used /{short} and /{long}");
            }
        }
    }

    #[test]
    fn a_power_of_two_divides_evenly() {
        // %4 on a /24 must be the same as /26.
        let by_count = report("10.0.0.0/24", &["%4"]);
        let by_length = report("10.0.0.0/24", &["/26"]);
        assert_eq!(
            by_count.parts[0]
                .blocks
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            by_length.splits[0]
                .subnets()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(by_count.parts[0].sizes(), vec![(26, 4)]);
    }

    #[test]
    fn dividing_after_a_carve_works_on_the_remainder() {
        let r = report("10.0.0.0/22", &["-24", "%5"]);
        assert_eq!(r.parts[0].source, Source::Remainder);
        let blocks: Vec<String> = r.parts[0].blocks.iter().map(ToString::to_string).collect();
        assert_eq!(
            blocks,
            vec![
                "10.0.1.0/25",
                "10.0.1.128/25",
                "10.0.2.0/25",
                "10.0.2.128/25",
                "10.0.3.0/24",
            ]
        );
        // The carved /24 is not among them.
        assert!(!blocks.contains(&"10.0.0.0/24".to_string()));
    }

    #[test]
    fn impossible_divisions_are_rejected_with_the_reason() {
        assert!(err("10.0.0.0/30", "%9").contains("4 is the most it holds"));
        assert!(errs("10.0.0.0/22", &["-24", "%1"]).contains("already 2 separate blocks"));
        assert!(err("10.0.0.0/24", "%70000").contains("limit 65,536"));
    }

    #[test]
    fn nonsensical_lengths_are_rejected_with_advice() {
        assert!(err("10.0.0.0/24", "/16").contains("+16"));
        assert!(err("10.0.0.0/24", "+30").contains("/30"));
        assert!(err("10.0.0.0/24", "/64").contains("IPv4"));
        assert!(err("10.0.0.0/24", "=2001:db8::1").contains("IPv6"));
        assert!(err("10.0.0.0/8", "-24*70000").contains("65,536"));
    }

    #[test]
    fn a_split_skips_remainder_blocks_that_are_too_small() {
        // Carving a /25 leaves a /25; a /24 split of the remainder has nothing
        // big enough to divide.
        let r = report("10.0.0.0/24", &["-25", "/24"]);
        assert_eq!(r.splits[0].blocks.len(), 0);
        assert_eq!(r.splits[0].too_small, 1);
    }

    #[test]
    fn shares_tile_the_space_exactly() {
        // The same property the carve map has to hold: every address of the
        // space goes to exactly one share, so the blocks abut with no gap and
        // no overlap. This is what makes a ratio an answer rather than a
        // suggestion.
        for (prefix, ops) in [
            ("10.0.0.0/24", vec!["%2:1:1"]),
            ("10.0.0.0/24", vec!["%3:1"]),
            ("10.0.0.0/24", vec!["%2:1"]),
            ("10.0.0.0/16", vec!["%7:1:1:1"]),
            ("10.0.0.0/16", vec!["%1:2:4:8"]),
            ("10.0.0.0/16", vec!["%5:3"]),
            ("2001:db8::/48", vec!["%2:1:1"]),
            ("2001:db8::/48", vec!["%9:5:3:1"]),
            // ... and over a ragged remainder, not just a whole prefix.
            ("10.0.0.0/16", vec!["-10.0.8.0/22", "%2:1:1"]),
            ("10.0.0.0/16", vec!["-24x3", "%3:1"]),
        ] {
            let r = report(prefix, &ops);
            let sh = &r.shares[0];
            let space: Vec<IpNet> = match &r.carve {
                Some(plan) => plan.free.clone(),
                None => vec![r.info.net],
            };

            let mut blocks: Vec<IpNet> = sh.granted.iter().flatten().copied().collect();
            blocks.sort();
            let addr = |n: &IpNet| to_u128(n.network());
            let end = |n: &IpNet| to_u128(n.broadcast());

            // Every share got something, and the blocks cover the space.
            assert!(
                sh.granted.iter().all(|g| !g.is_empty()),
                "{prefix} {ops:?} left a share with nothing"
            );
            let mut want: Vec<IpNet> = space.clone();
            want.sort();
            let mut cursor = 0;
            for block in &blocks {
                if cursor < want.len() && addr(block) > end(&want[cursor]) {
                    cursor += 1;
                }
                assert!(
                    cursor < want.len() && want[cursor].contains(block),
                    "{block} is outside the space being shared in {prefix} {ops:?}"
                );
            }
            for pair in blocks.windows(2) {
                let abuts = end(&pair[0]) + 1 == addr(&pair[1]);
                // A jump is only allowed where the free space itself jumped.
                let across_a_gap = want.iter().any(|b| end(b) == end(&pair[0]));
                assert!(
                    abuts || across_a_gap,
                    "{} and {} neither abut nor sit either side of a gap",
                    pair[0],
                    pair[1]
                );
            }
            // The totals add up to the space exactly.
            let got: u128 = blocks
                .iter()
                .map(|b| 1u128 << (b.max_prefix_len() - b.prefix_len()))
                .sum();
            let total: u128 = want
                .iter()
                .map(|b| 1u128 << (b.max_prefix_len() - b.prefix_len()))
                .sum();
            assert_eq!(got, total, "{prefix} {ops:?} does not add up");
        }
    }

    #[test]
    fn equal_shares_come_out_the_same_sizes_as_a_plain_count() {
        // `%1:1:1` is the ratio spelling of `%3`, so the two must divide the
        // space into the same blocks - which is also what pins the rounding
        // rule to the one `%M` already uses.
        // From two: a lone `%1` has no colon, so it is a count and not a
        // ratio at all.
        for m in 2..=12usize {
            let ratio = format!("%{}", vec!["1"; m].join(":"));
            let shares = report("10.0.0.0/20", &[&ratio]);
            let parts = report("10.0.0.0/20", &[&format!("%{m}")]);

            let mut a: Vec<u8> = shares.shares[0]
                .granted
                .iter()
                .flatten()
                .map(|n| n.prefix_len())
                .collect();
            let mut b: Vec<u8> = parts.parts[0]
                .blocks
                .iter()
                .map(|n| n.prefix_len())
                .collect();
            a.sort();
            b.sort();
            assert_eq!(a, b, "{ratio} and %{m} disagree");
        }
    }

    #[test]
    fn a_ratio_that_reduces_to_a_power_of_two_is_exact() {
        for (ratio, exact) in [
            ("%1:1", true),
            ("%2:1:1", true),
            ("%3:1", true),
            ("%1:2:4:8", false), // sums to 15
            ("%2:2:2:2", true),  // reduces to 1:1:1:1
            ("%6:2", true),      // reduces to 3:1
            ("%2:1", false),
            ("%1:1:1", false),
        ] {
            let r = report("10.0.0.0/16", &[ratio]);
            assert_eq!(r.shares[0].exact, exact, "{ratio}");
        }
    }

    #[test]
    fn an_inexact_ratio_reports_the_one_it_actually_got() {
        let r = report("10.0.0.0/24", &["%2:1"]);
        assert!(!r.shares[0].exact);
        assert_eq!(r.shares[0].achieved(), vec![3, 1]);
        // Reduced, so an exact split reports the ratio as asked for.
        let r = report("10.0.0.0/24", &["%4:2:2"]);
        assert!(r.shares[0].exact);
        assert_eq!(r.shares[0].achieved(), vec![2, 1, 1]);
    }

    #[test]
    fn shares_bigger_than_the_space_are_refused() {
        // A /30 is four addresses and the ratio wants five parts.
        assert!(errs("10.0.0.0/30", &["%2:1:1:1"]).contains("cannot be shared"));
        assert!(errs("10.0.0.0/32", &["%1:1"]).contains("cannot be shared"));
        // ... but exactly enough is fine.
        assert_eq!(
            report("10.0.0.0/30", &["%2:1:1"]).shares[0].granted.len(),
            3
        );
    }

    #[test]
    fn a_ragged_remainder_is_shared_rather_than_refused() {
        // Carving anything out of a large prefix leaves blocks of every size
        // down to the carve's own, and the granularity that divides them all
        // is fine. That is a normal thing to ask for, so it has to work.
        let r = report("10.0.0.0/8", &["-30", "%2:1:1"]);
        let sh = &r.shares[0];
        assert_eq!(sh.granted.len(), 3);
        // Not exact - the space is in pieces - but the ratio itself is fine,
        // and the note has to say which of the two it was.
        assert!(!sh.exact);
        assert!(sh.ratio_is_dyadic());
        // Too fine a unit to state as a ratio, so it falls back to percentages.
        assert!(sh.readable_ratio().is_none());
        let pct = sh.percentages();
        assert!((pct[0] - 50.0).abs() < 0.01, "{pct:?}");
        assert!((pct[1] - 25.0).abs() < 0.01, "{pct:?}");
    }

    #[test]
    fn the_two_kinds_of_inexact_are_told_apart() {
        // A ratio no prefix can express ...
        assert!(!report("10.0.0.0/24", &["%2:1"]).shares[0].ratio_is_dyadic());
        // ... versus one any prefix can, over a space that is not one.
        assert!(report("10.0.0.0/8", &["-30", "%2:1:1"]).shares[0].ratio_is_dyadic());
        assert!(report("10.0.0.0/24", &["%6:2"]).shares[0].ratio_is_dyadic());
    }

    #[test]
    fn shares_of_the_remainder_follow_the_carve() {
        let r = report("10.0.0.0/16", &["-10.0.8.0/22", "%1:1"]);
        assert_eq!(r.shares[0].source, Source::Remainder);
        // None of the shared blocks may overlap the carved one.
        let carved: IpNet = "10.0.8.0/22".parse().unwrap();
        for block in r.shares[0].granted.iter().flatten() {
            assert!(
                !carved.contains(block) && !block.contains(&carved),
                "{block} overlaps the carve"
            );
        }
    }

    #[test]
    fn zones_come_out_of_a_dot() {
        use crate::zones::Zones;
        let r = report("10.0.0.0/22", &["."]);
        let Zones::Aligned {
            boundary, count, ..
        } = r.zones[0]
        else {
            panic!("expected aligned zones");
        };
        assert_eq!((boundary, count.as_u128()), (24, Some(4)));

        // An explicit boundary that is not a label is a usage error, not a
        // silently rounded one.
        assert!(errs("10.0.0.0/22", &[".26"]).contains("delegation boundary"));
        assert!(errs("10.0.0.0/22", &[".8"]).contains("shorter than"));
    }
}
