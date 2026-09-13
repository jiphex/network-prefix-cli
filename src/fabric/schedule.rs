//! Which port on which switch reaches which - the list somebody takes to the
//! rack with a bag of cables.
//!
//! It is an iterator rather than a list because a mesh of a few hundred
//! switches is tens of thousands of links, and the usual thing to do with a
//! long one is pipe it into `head`. The port numbering is a running count per
//! switch, so it comes out in the same order the links do and never needs the
//! whole plan in memory at once.

use super::Topology;
use super::plan::{Arrangement, Plan};
use std::fmt;

/// One end of a link, which is a switch, a port on it, and a lane in that
/// port when the port is broken out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct End {
    /// Where the switch sits in the plan's own order, 1-based. Spines come
    /// before leaves, so this is not what the switch is called.
    pub switch: u64,
    /// What it is called: `sw` in a shape where every switch is alike, and
    /// `spine` or `leaf` where they are not, each numbered within its own
    /// population so `leaf1` is the first leaf rather than the first switch.
    pub prefix: &'static str,
    pub number: u64,
    pub port: u64,
    pub lane: Option<u64>,
}

impl End {
    /// What the switch at this end is called, without the port on it.
    pub fn name(&self) -> String {
        format!("{}{}", self.prefix, self.number)
    }

    /// The same, for a label that names the port separately.
    pub fn prefix_number(&self) -> String {
        self.name()
    }
}

impl fmt::Display for End {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}:{}", self.prefix, self.number, self.port)?;
        match self.lane {
            Some(lane) => write!(f, "/{lane}"),
            None => Ok(()),
        }
    }
}

/// One link, as a cable goes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Patch {
    pub a: End,
    pub b: End,
}

/// The pairs of switches a topology joins, in a fixed order: the upstream end
/// first where there is one, and its peers in order after it.
struct Links {
    topology: Topology,
    /// Every switch, spines included.
    total: u64,
    /// Where the first end stops: the switch count in a shape of peers, and
    /// the spine count in a leaf-spine.
    stop: u64,
    /// Where the second end starts over: 0 in a shape of peers, and the first
    /// leaf in a leaf-spine.
    floor: u64,
    per_pair: u64,
    a: u64,
    b: u64,
    done: u64,
}

impl Iterator for Links {
    type Item = (u64, u64);

    fn next(&mut self) -> Option<(u64, u64)> {
        if self.a >= self.stop {
            return None;
        }
        let pair = (self.a, self.b);
        self.done += 1;
        if self.done == self.per_pair {
            self.done = 0;
            self.advance();
        }
        Some(pair)
    }
}

impl Links {
    fn new(plan: &Plan) -> Links {
        let total = plan.switches;
        let spines = plan.spines;
        let mut links = Links {
            topology: plan.topology,
            total,
            stop: match plan.topology {
                Topology::LeafSpine => spines,
                // A ring of two would otherwise close back over the pair it
                // has already joined, which is the same link a second time.
                Topology::Ring if total == 2 => 1,
                _ => total,
            },
            floor: spines,
            per_pair: plan.per_pair,
            a: 0,
            b: 1,
            done: 0,
        };
        if plan.topology == Topology::LeafSpine {
            // It runs spine 1 to every leaf, then spine 2, and so on.
            links.b = spines;
        }
        links
    }

    fn advance(&mut self) {
        match self.topology {
            Topology::Mesh => {
                self.b += 1;
                if self.b >= self.total {
                    self.a += 1;
                    self.b = self.a + 1;
                }
                if self.b >= self.total {
                    self.a = self.stop;
                }
            }
            // Each switch joins the next, and the last closes back on the
            // first.
            Topology::Ring => {
                self.a += 1;
                self.b = (self.a + 1) % self.total;
            }
            // Switch 1 is the hub; every other switch hangs off it.
            Topology::Star => {
                self.b += 1;
                if self.b >= self.total {
                    self.a = self.stop;
                }
            }
            // Every leaf reaches every spine, a spine at a time.
            Topology::LeafSpine => {
                self.b += 1;
                if self.b >= self.total {
                    self.a += 1;
                    self.b = self.floor;
                }
            }
        }
    }
}

/// What a switch is called: `sw3` where every switch is alike, and `spine1`
/// or `leaf5` where they are not, each numbered within its own population.
pub fn name(plan: &Plan, index: u64) -> String {
    match plan.topology {
        Topology::LeafSpine if index < plan.spines => format!("spine{}", index + 1),
        Topology::LeafSpine => format!("leaf{}", index - plan.spines + 1),
        _ => format!("sw{}", index + 1),
    }
}

/// Every pair of switches the fabric joins, once each, whatever the per-pair
/// link count. A drawing wants the shape rather than the cables, and the
/// parallel links of a pair arrive together, so stepping over them is enough.
pub fn pairs(plan: &Plan) -> impl Iterator<Item = (u64, u64)> {
    Links::new(plan).step_by(plan.per_pair as usize)
}

/// The patch schedule, listing every link with the port each end connects
/// to.
pub struct Schedule {
    links: Links,
    topology: Topology,
    spines: u64,
    /// Lanes to a port, per switch, so a star's hub and a leaf-spine's spines
    /// can differ from what they reach.
    lanes: Vec<u64>,
    /// Link ends already assigned on each switch, which is what makes the
    /// next port number.
    used: Vec<u64>,
}

impl Schedule {
    pub fn new(plan: &Plan) -> Schedule {
        // A port is broken out on every switch, or only on the upstream
        // ones.
        let upstream = match plan.topology {
            Topology::LeafSpine => plan.spines,
            Topology::Star => 1,
            _ => 0,
        };
        let lanes = (0..plan.switches)
            .map(|i| match plan.arrangement {
                Arrangement::Straight => 1,
                Arrangement::SplitBothEnds => plan.lanes,
                Arrangement::SplitUpstream if i < upstream => plan.lanes,
                Arrangement::SplitUpstream => 1,
            })
            .collect();
        Schedule {
            links: Links::new(plan),
            topology: plan.topology,
            spines: plan.spines,
            lanes,
            used: vec![0; plan.switches as usize],
        }
    }

    fn end(&mut self, switch: u64) -> End {
        let i = switch as usize;
        let lanes = self.lanes[i];
        let n = self.used[i];
        self.used[i] += 1;
        let (prefix, number) = match self.topology {
            Topology::LeafSpine if switch < self.spines => ("spine", switch + 1),
            Topology::LeafSpine => ("leaf", switch - self.spines + 1),
            _ => ("sw", switch + 1),
        };
        End {
            switch: switch + 1,
            prefix,
            number,
            port: n / lanes + 1,
            lane: (lanes > 1).then_some(n % lanes + 1),
        }
    }
}

impl Iterator for Schedule {
    type Item = Patch;

    fn next(&mut self) -> Option<Patch> {
        let (a, b) = self.links.next()?;
        Some(Patch {
            a: self.end(a),
            b: self.end(b),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fabric::plan::{self, Problem};

    fn plan_of(fabric: &str) -> Plan {
        let r = crate::fabric::dsl::parse(fabric).expect("the fabric parses");
        plan::build(
            r.switches,
            &r.ops,
            plan::Options {
                media: r.media,
                shape: r.shape,
                schedule: false,
            },
        )
        .map_err(|e: Problem| e.to_string())
        .expect("plans")
        .plan
    }

    /// A fabric of a given shape and size, for the sweeps below.
    fn fabric(shape: &str, switches: u64, link: &str, panel: &str) -> String {
        match shape {
            "star" => format!("1 hub{panel} {link} {} spokes", switches - 1),
            "leaf-spine" => format!("2 spines{panel} {link} {switches} leaves"),
            _ => format!("{switches} switches{panel} {link} {shape}"),
        }
    }

    fn lines(fabric: &str) -> Vec<String> {
        Schedule::new(&plan_of(fabric))
            .map(|p| format!("{} {}", p.a, p.b))
            .collect()
    }

    #[test]
    fn a_mesh_lists_every_pair_once() {
        assert_eq!(
            lines("4 switches -- mesh"),
            [
                "sw1:1 sw2:1",
                "sw1:2 sw3:1",
                "sw1:3 sw4:1",
                "sw2:2 sw3:2",
                "sw2:3 sw4:2",
                "sw3:3 sw4:3",
            ]
        );
    }

    #[test]
    fn a_ring_closes_and_a_star_fans_out() {
        assert_eq!(
            lines("4 switches -- ring"),
            ["sw1:1 sw2:1", "sw2:2 sw3:1", "sw3:2 sw4:1", "sw4:2 sw1:2"]
        );
        assert_eq!(
            lines("1 hub -- 3 spokes"),
            ["sw1:1 sw2:1", "sw1:2 sw3:1", "sw1:3 sw4:1"]
        );
        // Two switches in a ring are one link, not the same link twice.
        assert_eq!(lines("2 switches -- ring"), ["sw1:1 sw2:1"]);
    }

    #[test]
    fn lanes_fill_a_port_before_the_next_one_is_used() {
        assert_eq!(
            lines("6 switches[400G] -100G- mesh"),
            [
                "sw1:1/1 sw2:1/1",
                "sw1:1/2 sw3:1/1",
                "sw1:1/3 sw4:1/1",
                "sw1:1/4 sw5:1/1",
                "sw1:2/1 sw6:1/1",
                "sw2:1/2 sw3:1/2",
                "sw2:1/3 sw4:1/2",
                "sw2:1/4 sw5:1/2",
                "sw2:2/1 sw6:1/2",
                "sw3:1/3 sw4:1/3",
                "sw3:1/4 sw5:1/3",
                "sw3:2/1 sw6:1/3",
                "sw4:1/4 sw5:1/4",
                "sw4:2/1 sw6:1/4",
                "sw5:2/1 sw6:2/1",
            ]
        );
    }

    #[test]
    fn only_the_hub_of_a_star_is_broken_out() {
        assert_eq!(
            lines("1 hub[400G] -100G- 5 spokes"),
            [
                "sw1:1/1 sw2:1",
                "sw1:1/2 sw3:1",
                "sw1:1/3 sw4:1",
                "sw1:1/4 sw5:1",
                "sw1:2/1 sw6:1",
            ]
        );
    }

    #[test]
    fn a_leaf_spine_wires_every_leaf_to_every_spine() {
        assert_eq!(
            lines("2 spines -- 3 leaves"),
            [
                "spine1:1 leaf1:1",
                "spine1:2 leaf2:1",
                "spine1:3 leaf3:1",
                "spine2:1 leaf1:2",
                "spine2:2 leaf2:2",
                "spine2:3 leaf3:2",
            ]
        );
        // Only the spines break out, and a spine port carries four leaves.
        assert_eq!(
            lines("1 spines[400G] -100G- 5 leaves"),
            [
                "spine1:1/1 leaf1:1",
                "spine1:1/2 leaf2:1",
                "spine1:1/3 leaf3:1",
                "spine1:1/4 leaf4:1",
                "spine1:2/1 leaf5:1",
            ]
        );
    }

    #[test]
    fn parallel_links_are_listed_one_after_another() {
        assert_eq!(
            lines("3 switches -2x- mesh"),
            [
                "sw1:1 sw2:1",
                "sw1:2 sw2:2",
                "sw1:3 sw3:1",
                "sw1:4 sw3:2",
                "sw2:3 sw3:3",
                "sw2:4 sw3:4",
            ]
        );
    }

    /// The schedule and the plan have to be the same fabric: as many lines as
    /// the plan counted links, and as many ends on each switch as it counted
    /// ports. This is what catches a topology whose iterator and whose
    /// arithmetic disagree about what it joins.
    #[test]
    fn the_schedule_is_the_plan_it_came_from() {
        for (shape, link, panel, tail) in [
            ("mesh", "--", "", ""),
            ("ring", "--", "", ""),
            ("star", "--", "", ""),
            ("mesh", "-2x-", "", ""),
            ("ring", "-3x-", "", ""),
            ("mesh", "-100G-", "[400G]", ""),
            ("star", "-100G-", "[400G]", ""),
            ("ring", "-100G-", "[800G]", ""),
            ("leaf-spine", "--", "", ""),
            ("leaf-spine", "-2x-", "", ""),
            ("leaf-spine", "-100G-", "[400G]", ""),
            ("leaf-spine", "-100G-", "[400G]", "-25G- 48 servers"),
        ] {
            for count in 2..14u64 {
                let args = format!("{} {tail}", fabric(shape, count, link, panel));
                let plan = plan_of(&args);
                let switches = plan.switches;
                let spines = plan.spines;
                let mut ports = vec![0u64; switches as usize];
                let mut ends = vec![0u64; switches as usize];
                let mut links = 0;
                for patch in Schedule::new(&plan) {
                    links += 1;
                    for end in [patch.a, patch.b] {
                        let i = end.switch as usize - 1;
                        ports[i] = ports[i].max(end.port);
                        ends[i] += 1;
                    }
                }
                assert_eq!(links, plan.links, "{count} {args:?}: link count");
                for side in &plan.sides {
                    // Spines are numbered first, then leaves. A star's hub is
                    // switch 1 and its spokes are the rest, and in the shapes
                    // with one kind of switch they are all alike.
                    let range: Vec<usize> = match side.role {
                        plan::Role::Hub => vec![0],
                        plan::Role::Spoke => (1..switches as usize).collect(),
                        plan::Role::Spine => (0..spines as usize).collect(),
                        plan::Role::Leaf => (spines as usize..switches as usize).collect(),
                        plan::Role::Every => (0..switches as usize).collect(),
                    };
                    for i in range {
                        assert_eq!(ends[i], side.degree, "{count} {args:?}: ends on sw{i}");
                        assert_eq!(ports[i], side.ports, "{count} {args:?}: ports on sw{i}");
                    }
                }
            }
        }
    }

    /// Nothing is plugged into a port twice: a lane carries one link end.
    #[test]
    fn every_lane_is_used_once() {
        for (shape, link, panel) in [
            ("mesh", "--", ""),
            ("mesh", "-100G-", "[400G]"),
            ("ring", "-2x-", ""),
            ("leaf-spine", "-100G-", "[400G]"),
        ] {
            for switches in 2..12u64 {
                let args = fabric(shape, switches, link, panel);
                let plan = plan_of(&args);
                let mut seen = std::collections::HashSet::new();
                for patch in Schedule::new(&plan) {
                    for end in [patch.a, patch.b] {
                        assert!(seen.insert(end.to_string()), "{end} twice");
                    }
                }
                assert_eq!(seen.len() as u64, 2 * plan.links);
            }
        }
    }

    /// A big mesh must not be built before its first line can be printed.
    #[test]
    fn a_long_schedule_starts_immediately() {
        let plan = plan_of("4096 switches -- mesh");
        assert_eq!(plan.links, 8_386_560);
        let first: Vec<String> = Schedule::new(&plan)
            .take(2)
            .map(|p| format!("{} {}", p.a, p.b))
            .collect();
        assert_eq!(first, ["sw1:1 sw2:1", "sw1:2 sw3:1"]);
    }
}
