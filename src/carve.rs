//! Carving subnets out of a parent prefix.
//!
//! The allocator keeps a free list of disjoint, aligned blocks and hands out
//! space best-fit: the smallest free block that can still hold the request,
//! lowest address first. That is how you would do it on a whiteboard, and it
//! leaves the big blocks intact for the big requests.
//!
//! Fixed requests (carve out *this* subnet) are honoured before floating ones
//! (carve out *a* /56), because a fixed request has nowhere else to go.

use crate::num::Count;
use ipnet::IpNet;

/// The largest number of subnets a single request may ask for. Anything
/// bigger is a split, not a carve.
pub const MAX_REQUEST_COUNT: u64 = 65_536;

#[derive(Debug, Clone)]
pub enum Request {
    /// "give me a subnet of this length, anywhere it fits"
    ///
    /// One subnet per request. A request for several is expanded by the
    /// caller into one of these each, so that every allocation gets its own
    /// outcome: a single request standing for several could only report one
    /// of them, and the rest would silently vanish from the results while
    /// still being taken out of the free list.
    Floating(u8),
    /// "reserve exactly this subnet"
    Fixed(IpNet),
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Granted(IpNet),
    /// The request can never be satisfied from this parent (wrong family,
    /// bigger than the parent, outside it).
    Impossible(String),
    /// It would have fit in an empty parent, but the space is gone.
    Exhausted,
}

#[derive(Debug, Clone)]
pub struct Grant {
    /// How the request was written, for echoing back to the user.
    pub label: String,
    pub outcome: Outcome,
}

#[derive(Debug)]
pub struct Plan {
    pub parent: IpNet,
    pub grants: Vec<Grant>,
    /// Remaining space, aggregated into the fewest possible blocks.
    pub free: Vec<IpNet>,
}

/// One block of the parent, as it appears in the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub net: IpNet,
    /// True when this block was handed out rather than left free.
    pub carved: bool,
}

impl Plan {
    /// Every block of the parent in address order, marked with whether it was
    /// carved out - a picture of where the allocations landed rather than two
    /// separate lists to cross-reference by eye.
    ///
    /// The allocations and the free blocks tile the parent exactly between
    /// them, so no address is listed twice or missed.
    pub fn map(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .granted()
            .map(|n| Row {
                net: *n,
                carved: true,
            })
            .chain(self.free.iter().map(|n| Row {
                net: *n,
                carved: false,
            }))
            .collect();
        // Disjoint blocks, so ordering by network address is a total order.
        rows.sort_by_key(|r| r.net);
        rows
    }

    pub fn granted(&self) -> impl Iterator<Item = &IpNet> {
        self.grants.iter().filter_map(|g| match &g.outcome {
            Outcome::Granted(n) => Some(n),
            _ => None,
        })
    }

    pub fn all_granted(&self) -> bool {
        self.grants
            .iter()
            .all(|g| matches!(g.outcome, Outcome::Granted(_)))
    }

    /// Address counts of each remaining free block.
    pub fn free_counts(&self) -> Vec<Count> {
        self.free
            .iter()
            .map(|n| Count::pow2(u32::from(n.max_prefix_len() - n.prefix_len())))
            .collect()
    }

    /// The biggest single block still available.
    pub fn largest_free(&self) -> Option<IpNet> {
        self.free.iter().min_by_key(|n| n.prefix_len()).copied()
    }
}

/// Run every request against `parent`, in the order given, and report both the
/// allocations and what is left.
pub fn plan(parent: IpNet, requests: &[Request]) -> Plan {
    let parent = parent.trunc();
    let mut free = vec![parent];
    // Slots keep command-line order for display, while we service fixed
    // requests first so they cannot be squeezed out by floating ones.
    let mut grants: Vec<Option<Grant>> = vec![None; requests.len()].into_iter().collect();

    for pass in [Pass::Fixed, Pass::Floating] {
        for (i, req) in requests.iter().enumerate() {
            match (pass, req) {
                (Pass::Fixed, Request::Fixed(net)) => {
                    grants[i] = Some(Grant {
                        label: net.to_string(),
                        outcome: take_exact(&mut free, parent, *net),
                    });
                }
                (Pass::Floating, Request::Floating(len)) => {
                    grants[i] = Some(Grant {
                        label: format!("/{len}"),
                        outcome: take(&mut free, parent, *len),
                    });
                }
                _ => {}
            }
        }
    }

    free.sort();
    Plan {
        parent,
        grants: grants
            .into_iter()
            .map(|g| g.expect("every slot filled"))
            .collect(),
        free: IpNet::aggregate(&free),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Pass {
    Fixed,
    Floating,
}

/// Best-fit allocation of a single subnet of length `len`.
fn take(free: &mut Vec<IpNet>, parent: IpNet, len: u8) -> Outcome {
    if len > parent.max_prefix_len() {
        return Outcome::Impossible(format!(
            "/{len} is not a valid length for {}",
            family(&parent)
        ));
    }
    if len < parent.prefix_len() {
        return Outcome::Impossible(format!("/{len} is larger than the parent {parent}"));
    }
    // `free` is kept sorted, so the first block of the best size is also the
    // lowest-addressed one: allocations march up from the bottom.
    let mut best: Option<usize> = None;
    for (i, block) in free.iter().enumerate() {
        if block.prefix_len() <= len
            && best.is_none_or(|j| block.prefix_len() > free[j].prefix_len())
        {
            best = Some(i);
        }
    }
    let Some(i) = best else {
        return Outcome::Exhausted;
    };
    let mut cur = free.remove(i);
    while cur.prefix_len() < len {
        let (low, high) = halves(&cur);
        free.push(high);
        cur = low;
    }
    free.sort();
    Outcome::Granted(cur)
}

/// Reserve one specific subnet, splitting whichever free block holds it.
fn take_exact(free: &mut Vec<IpNet>, parent: IpNet, target: IpNet) -> Outcome {
    let target = target.trunc();
    if target.addr().is_ipv4() != parent.addr().is_ipv4() {
        return Outcome::Impossible(format!("{target} is not the same family as {parent}"));
    }
    if !parent.contains(&target) {
        return Outcome::Impossible(format!("{target} is not inside {parent}"));
    }
    let Some(i) = free.iter().position(|b| b.contains(&target)) else {
        return Outcome::Exhausted;
    };
    let block = free.remove(i);
    free.extend(complement(block, target));
    free.sort();
    Outcome::Granted(target)
}

/// Split a block in two.
pub fn halves(net: &IpNet) -> (IpNet, IpNet) {
    let mut it = net
        .subnets(net.prefix_len() + 1)
        .expect("a block shorter than a host route always splits");
    let low = it.next().expect("first half");
    let high = it.next().expect("second half");
    (low, high)
}

/// The aligned blocks covering `container` minus `target`, where `target` is
/// inside `container`. Splitting down towards the target peels off one sibling
/// per level, which is exactly the minimal covering set.
pub fn complement(container: IpNet, target: IpNet) -> Vec<IpNet> {
    let mut out = Vec::new();
    let mut cur = container;
    while cur.prefix_len() < target.prefix_len() {
        let (low, high) = halves(&cur);
        if low.contains(&target) {
            out.push(high);
            cur = low;
        } else {
            out.push(low);
            cur = high;
        }
    }
    out
}

pub fn family(net: &IpNet) -> &'static str {
    if net.addr().is_ipv4() { "IPv4" } else { "IPv6" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(s: &str) -> IpNet {
        s.parse().unwrap()
    }

    fn granted(plan: &Plan) -> Vec<String> {
        plan.granted().map(|n| n.to_string()).collect()
    }

    fn free(plan: &Plan) -> Vec<String> {
        plan.free.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn carves_the_example_from_the_readme() {
        let p = plan(
            net("2001:db8::/52"),
            &[
                Request::Floating(56),
                Request::Floating(64),
                Request::Floating(64),
            ],
        );
        assert_eq!(
            granted(&p),
            vec![
                "2001:db8::/56",
                "2001:db8:0:100::/64",
                "2001:db8:0:101::/64"
            ]
        );
        // The two /64s share a /56, so the remainder starts at the next /56.
        assert!(p.free.iter().all(|f| !f.contains(&net("2001:db8::/56"))));
        assert!(p.all_granted());
    }

    #[test]
    fn best_fit_keeps_large_blocks_whole() {
        // Ask small-then-large: the /64 must not chop into the block the /56
        // will need, because the /56 is serviced first by size.
        let p = plan(
            net("2001:db8::/52"),
            &[Request::Floating(64), Request::Floating(56)],
        );
        let g = granted(&p);
        // Both fit, and the /56 is aligned on a /56 boundary.
        assert_eq!(g.len(), 2);
        let fifty_six: IpNet = g[1].parse().unwrap();
        assert_eq!(fifty_six.trunc(), fifty_six);
    }

    #[test]
    fn fixed_requests_win_over_floating_ones() {
        // The floating /64 would otherwise take 10.0.0.0/24's space first.
        let p = plan(
            net("10.0.0.0/22"),
            &[Request::Floating(24), Request::Fixed(net("10.0.0.0/24"))],
        );
        assert!(p.all_granted());
        assert_eq!(granted(&p), vec!["10.0.1.0/24", "10.0.0.0/24"]);
    }

    #[test]
    fn the_map_places_the_carve_among_the_blocks_around_it() {
        let p = plan(
            net("2001:db8::/56"),
            &[Request::Fixed(net("2001:db8:0:cc::/64"))],
        );
        let rows: Vec<String> = p
            .map()
            .iter()
            .map(|r| format!("{}{}", if r.carved { "*" } else { " " }, r.net))
            .collect();
        assert_eq!(
            rows,
            vec![
                " 2001:db8::/57",
                " 2001:db8:0:80::/58",
                " 2001:db8:0:c0::/61",
                " 2001:db8:0:c8::/62",
                "*2001:db8:0:cc::/64",
                " 2001:db8:0:cd::/64",
                " 2001:db8:0:ce::/63",
                " 2001:db8:0:d0::/60",
                " 2001:db8:0:e0::/59",
            ]
        );
    }

    #[test]
    fn the_map_tiles_the_parent_exactly() {
        // Every address in the parent appears in exactly one row: the sizes
        // must add up, and consecutive rows must abut with no gap or overlap.
        for (parent, reqs) in [
            (
                "2001:db8::/56",
                vec![Request::Fixed(net("2001:db8:0:cc::/64"))],
            ),
            (
                "10.0.0.0/16",
                vec![
                    Request::Fixed(net("10.0.8.0/22")),
                    Request::Floating(24),
                    Request::Floating(24),
                    Request::Floating(24),
                    Request::Floating(24),
                ],
            ),
            ("10.0.0.0/24", vec![Request::Floating(24)]),
            (
                "10.0.0.0/22",
                vec![
                    Request::Floating(30),
                    Request::Floating(30),
                    Request::Floating(30),
                ],
            ),
        ] {
            let parent: IpNet = parent.parse().unwrap();
            let p = plan(parent, &reqs);
            let rows = p.map();
            assert!(!rows.is_empty(), "{parent} produced an empty map");

            let addr = |n: &IpNet| crate::report::to_u128(n.network());
            let end = |n: &IpNet| crate::report::to_u128(n.broadcast());

            assert_eq!(addr(&rows[0].net), addr(&parent), "map starts late");
            assert_eq!(
                end(&rows[rows.len() - 1].net),
                end(&parent),
                "map ends early"
            );
            for pair in rows.windows(2) {
                assert_eq!(
                    end(&pair[0].net) + 1,
                    addr(&pair[1].net),
                    "{} and {} do not abut",
                    pair[0].net,
                    pair[1].net
                );
            }
        }
    }

    #[test]
    fn remainder_is_aggregated() {
        let p = plan(net("10.0.0.0/24"), &[Request::Floating(25)]);
        assert_eq!(free(&p), vec!["10.0.0.128/25"]);
    }

    #[test]
    fn exhaustion_is_reported_not_panicked() {
        let p = plan(
            net("10.0.0.0/24"),
            &[Request::Floating(24), Request::Floating(30)],
        );
        assert!(!p.all_granted());
        assert!(matches!(p.grants[1].outcome, Outcome::Exhausted));
        assert!(p.free.is_empty());
    }

    #[test]
    fn requests_bigger_than_the_parent_are_impossible() {
        let p = plan(net("10.0.0.0/24"), &[Request::Floating(16)]);
        assert!(matches!(p.grants[0].outcome, Outcome::Impossible(_)));
    }

    #[test]
    fn a_v6_length_against_a_v4_parent_is_impossible() {
        let p = plan(net("10.0.0.0/8"), &[Request::Floating(64)]);
        assert!(matches!(p.grants[0].outcome, Outcome::Impossible(_)));
    }

    #[test]
    fn excluding_something_outside_the_parent_is_impossible() {
        let p = plan(net("10.0.0.0/24"), &[Request::Fixed(net("192.168.0.0/24"))]);
        assert!(matches!(p.grants[0].outcome, Outcome::Impossible(_)));
    }

    #[test]
    fn excluding_the_same_subnet_twice_exhausts() {
        let p = plan(
            net("10.0.0.0/22"),
            &[
                Request::Fixed(net("10.0.1.0/24")),
                Request::Fixed(net("10.0.1.0/24")),
            ],
        );
        assert!(matches!(p.grants[0].outcome, Outcome::Granted(_)));
        assert!(matches!(p.grants[1].outcome, Outcome::Exhausted));
    }

    #[test]
    fn complement_covers_everything_else() {
        let c = complement(net("10.0.0.0/22"), net("10.0.1.0/24"));
        assert_eq!(
            c.iter().map(|n| n.to_string()).collect::<Vec<_>>(),
            // Peeled off outermost sibling first; callers sort.
            vec!["10.0.2.0/23", "10.0.0.0/24"]
        );
    }
}
