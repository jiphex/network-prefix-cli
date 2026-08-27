//! Carving subnets out of a parent prefix.
//!
//! The allocator keeps a free list of disjoint, aligned blocks and hands out
//! space best-fit: the smallest free block that can still hold the request,
//! lowest address first. That is how you would do it on a whiteboard, and it
//! leaves the big blocks intact for the big requests.
//!
//! Fixed requests (carve out *this* subnet) are honoured before floating ones
//! (carve out *a* /56), because a fixed request has nowhere else to go.
//!
//! Floating requests fill from the bottom of the parent by default. `Top`
//! fills from the other end instead, which is how an infrastructure block is
//! usually taken - down from the top, so that it grows towards the customer
//! allocations coming up from the bottom rather than into them.

use crate::num::Count;
use ipnet::IpNet;

/// The largest number of subnets a single request may ask for. Anything
/// bigger is a split, not a carve.
pub const MAX_REQUEST_COUNT: u64 = 65_536;

/// Which end of the parent floating requests are filled from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// Lowest free block first.
    #[default]
    Bottom,
    /// Highest free block first.
    Top,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub kind: Kind,
    /// A name for this allocation, carried through to the table and the map.
    pub label: Option<String>,
}

impl Request {
    pub fn floating(len: u8) -> Request {
        Request {
            kind: Kind::Floating(len),
            label: None,
        }
    }

    pub fn fixed(net: IpNet) -> Request {
        Request {
            kind: Kind::Fixed(net),
            label: None,
        }
    }

    pub fn named(self, label: Option<String>) -> Request {
        Request { label, ..self }
    }
}

#[derive(Debug, Clone)]
pub enum Kind {
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
    /// The name the request was given, if any.
    pub name: Option<String>,
    pub outcome: Outcome,
}

#[derive(Debug)]
pub struct Plan {
    pub parent: IpNet,
    pub direction: Direction,
    pub grants: Vec<Grant>,
    /// Remaining space, aggregated into the fewest possible blocks.
    pub free: Vec<IpNet>,
}

/// One block of the parent, as it appears in the map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub net: IpNet,
    /// True when this block was handed out rather than left free.
    pub carved: bool,
    /// The name of the allocation sitting here, for a carved row.
    pub label: Option<String>,
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
            .grants
            .iter()
            .filter_map(|g| match &g.outcome {
                Outcome::Granted(n) => Some(Row {
                    net: *n,
                    carved: true,
                    label: g.name.clone(),
                }),
                _ => None,
            })
            .chain(self.free.iter().map(|n| Row {
                net: *n,
                carved: false,
                label: None,
            }))
            .collect();
        // Disjoint blocks, so ordering by network address is a total order.
        rows.sort_by_key(|r| r.net);
        rows
    }

    /// True when any allocation was given a name, so the renderers know
    /// whether a name column is worth its width.
    pub fn any_named(&self) -> bool {
        self.grants.iter().any(|g| g.name.is_some())
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
pub fn plan(parent: IpNet, requests: &[Request], direction: Direction) -> Plan {
    let parent = parent.trunc();
    let mut free = vec![parent];
    // Slots keep command-line order for display, while we service fixed
    // requests first so they cannot be squeezed out by floating ones.
    let mut grants: Vec<Option<Grant>> = vec![None; requests.len()].into_iter().collect();

    for pass in [Pass::Fixed, Pass::Floating] {
        for (i, req) in requests.iter().enumerate() {
            let (label, outcome) = match (pass, &req.kind) {
                (Pass::Fixed, Kind::Fixed(net)) => {
                    (net.to_string(), take_exact(&mut free, parent, *net))
                }
                (Pass::Floating, Kind::Floating(len)) => {
                    (format!("/{len}"), take(&mut free, parent, *len, direction))
                }
                _ => continue,
            };
            grants[i] = Some(Grant {
                label,
                name: req.label.clone(),
                outcome,
            });
        }
    }

    free.sort();
    Plan {
        parent,
        direction,
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
fn take(free: &mut Vec<IpNet>, parent: IpNet, len: u8, direction: Direction) -> Outcome {
    if len > parent.max_prefix_len() {
        return Outcome::Impossible(format!(
            "/{len} is not a valid length for {}",
            family(&parent)
        ));
    }
    if len < parent.prefix_len() {
        return Outcome::Impossible(format!("/{len} is larger than the parent {parent}"));
    }
    // `free` is kept sorted, so among equally good blocks the first is the
    // lowest-addressed one and the last is the highest. Keeping the earlier
    // one on a tie marches allocations up from the bottom; keeping the later
    // one marches them down from the top.
    let mut best: Option<usize> = None;
    for (i, block) in free.iter().enumerate() {
        let better = |j: usize| match direction {
            Direction::Bottom => block.prefix_len() > free[j].prefix_len(),
            Direction::Top => block.prefix_len() >= free[j].prefix_len(),
        };
        if block.prefix_len() <= len && best.is_none_or(better) {
            best = Some(i);
        }
    }
    let Some(i) = best else {
        return Outcome::Exhausted;
    };
    // Splitting down towards the request takes the half nearest the end being
    // filled from, so the allocation lands at that end of the block rather
    // than merely in the block nearest it.
    let mut cur = free.remove(i);
    while cur.prefix_len() < len {
        let (low, high) = halves(&cur);
        match direction {
            Direction::Bottom => {
                free.push(high);
                cur = low;
            }
            Direction::Top => {
                free.push(low);
                cur = high;
            }
        }
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
                Request::floating(56),
                Request::floating(64),
                Request::floating(64),
            ],
            Direction::Bottom,
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
            &[Request::floating(64), Request::floating(56)],
            Direction::Bottom,
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
            &[Request::floating(24), Request::fixed(net("10.0.0.0/24"))],
            Direction::Bottom,
        );
        assert!(p.all_granted());
        assert_eq!(granted(&p), vec!["10.0.1.0/24", "10.0.0.0/24"]);
    }

    #[test]
    fn the_map_places_the_carve_among_the_blocks_around_it() {
        let p = plan(
            net("2001:db8::/56"),
            &[Request::fixed(net("2001:db8:0:cc::/64"))],
            Direction::Bottom,
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
                vec![Request::fixed(net("2001:db8:0:cc::/64"))],
            ),
            (
                "10.0.0.0/16",
                vec![
                    Request::fixed(net("10.0.8.0/22")),
                    Request::floating(24),
                    Request::floating(24),
                    Request::floating(24),
                    Request::floating(24),
                ],
            ),
            ("10.0.0.0/24", vec![Request::floating(24)]),
            (
                "10.0.0.0/22",
                vec![
                    Request::floating(30),
                    Request::floating(30),
                    Request::floating(30),
                ],
            ),
        ] {
            let parent: IpNet = parent.parse().unwrap();
            for direction in [Direction::Bottom, Direction::Top] {
                check_tiling(parent, &reqs, direction);
            }
        }
    }

    fn check_tiling(parent: IpNet, reqs: &[Request], direction: Direction) {
        {
            let p = plan(parent, reqs, direction);
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
    fn filling_from_the_top_starts_at_the_top() {
        let p = plan(
            net("10.0.0.0/16"),
            &[Request::floating(24), Request::floating(24)],
            Direction::Top,
        );
        assert_eq!(granted(&p), vec!["10.0.255.0/24", "10.0.254.0/24"]);
        // ... and leaves the bottom whole, which is the point of asking.
        assert_eq!(p.largest_free().unwrap().to_string(), "10.0.0.0/17");
    }

    #[test]
    fn the_two_directions_are_mirror_images() {
        // The same requests filled from either end must produce allocations
        // that are reflections of each other about the middle of the parent.
        let parent = net("10.0.0.0/16");
        let reqs = [
            Request::floating(24),
            Request::floating(22),
            Request::floating(30),
            Request::floating(24),
        ];
        let bottom = plan(parent, &reqs, Direction::Bottom);
        let top = plan(parent, &reqs, Direction::Top);

        let first = crate::report::to_u128(parent.network());
        let last = crate::report::to_u128(parent.broadcast());
        for (b, t) in bottom.granted().zip(top.granted()) {
            assert_eq!(b.prefix_len(), t.prefix_len());
            // Reflecting a block about the parent swaps its ends, so the
            // mirror of its first address is its last.
            assert_eq!(
                first + last - crate::report::to_u128(b.network()),
                crate::report::to_u128(t.broadcast()),
                "{b} and {t} are not reflections"
            );
        }
    }

    #[test]
    fn a_direction_does_not_move_a_fixed_request() {
        // Fixed requests have nowhere else to go, whichever end is being
        // filled from.
        for direction in [Direction::Bottom, Direction::Top] {
            let p = plan(
                net("10.0.0.0/16"),
                &[Request::fixed(net("10.0.8.0/22"))],
                direction,
            );
            assert_eq!(granted(&p), vec!["10.0.8.0/22"]);
        }
    }

    #[test]
    fn a_name_rides_along_to_the_grant_and_the_map() {
        let p = plan(
            net("10.0.0.0/22"),
            &[
                Request::floating(24).named(Some("dmz".into())),
                Request::fixed(net("10.0.3.0/24")).named(Some("legacy".into())),
            ],
            Direction::Bottom,
        );
        assert!(p.any_named());
        assert_eq!(p.grants[0].name.as_deref(), Some("dmz"));
        assert_eq!(p.grants[1].name.as_deref(), Some("legacy"));

        let named: Vec<(String, Option<String>)> = p
            .map()
            .into_iter()
            .filter(|r| r.carved)
            .map(|r| (r.net.to_string(), r.label))
            .collect();
        // Best fit puts the floating /24 in the /24-sized hole the fixed
        // request left behind, not at the bottom of the parent.
        assert_eq!(
            named,
            vec![
                ("10.0.2.0/24".to_string(), Some("dmz".to_string())),
                ("10.0.3.0/24".to_string(), Some("legacy".to_string())),
            ]
        );
        // Nothing named means no name column anywhere.
        assert!(
            !plan(
                net("10.0.0.0/22"),
                &[Request::floating(24)],
                Direction::Bottom
            )
            .any_named()
        );
    }

    #[test]
    fn remainder_is_aggregated() {
        let p = plan(
            net("10.0.0.0/24"),
            &[Request::floating(25)],
            Direction::Bottom,
        );
        assert_eq!(free(&p), vec!["10.0.0.128/25"]);
    }

    #[test]
    fn exhaustion_is_reported_not_panicked() {
        let p = plan(
            net("10.0.0.0/24"),
            &[Request::floating(24), Request::floating(30)],
            Direction::Bottom,
        );
        assert!(!p.all_granted());
        assert!(matches!(p.grants[1].outcome, Outcome::Exhausted));
        assert!(p.free.is_empty());
    }

    #[test]
    fn requests_bigger_than_the_parent_are_impossible() {
        let p = plan(
            net("10.0.0.0/24"),
            &[Request::floating(16)],
            Direction::Bottom,
        );
        assert!(matches!(p.grants[0].outcome, Outcome::Impossible(_)));
    }

    #[test]
    fn a_v6_length_against_a_v4_parent_is_impossible() {
        let p = plan(
            net("10.0.0.0/8"),
            &[Request::floating(64)],
            Direction::Bottom,
        );
        assert!(matches!(p.grants[0].outcome, Outcome::Impossible(_)));
    }

    #[test]
    fn excluding_something_outside_the_parent_is_impossible() {
        let p = plan(
            net("10.0.0.0/24"),
            &[Request::fixed(net("192.168.0.0/24"))],
            Direction::Bottom,
        );
        assert!(matches!(p.grants[0].outcome, Outcome::Impossible(_)));
    }

    #[test]
    fn excluding_the_same_subnet_twice_exhausts() {
        let p = plan(
            net("10.0.0.0/22"),
            &[
                Request::fixed(net("10.0.1.0/24")),
                Request::fixed(net("10.0.1.0/24")),
            ],
            Direction::Bottom,
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
