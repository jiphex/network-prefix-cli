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

/// One end of a link: a switch, a port on it, and a lane in that port when
/// the port is broken out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct End {
    pub switch: u64,
    pub port: u64,
    pub lane: Option<u64>,
}

impl fmt::Display for End {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.lane {
            Some(lane) => write!(f, "sw{}:{}/{lane}", self.switch, self.port),
            None => write!(f, "sw{}:{}", self.switch, self.port),
        }
    }
}

/// One link, as a cable goes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Patch {
    pub a: End,
    pub b: End,
}

/// The pairs of switches a topology joins, in a fixed order: lowest switch
/// first, and its peers in order after it.
struct Links {
    switches: u64,
    topology: Topology,
    per_pair: u64,
    a: u64,
    b: u64,
    done: u64,
}

impl Iterator for Links {
    type Item = (u64, u64);

    fn next(&mut self) -> Option<(u64, u64)> {
        if self.a >= self.switches {
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
        let mut links = Links {
            switches: plan.switches,
            topology: plan.topology,
            per_pair: plan.per_pair,
            a: 0,
            b: 1,
            done: 0,
        };
        // A ring of two would otherwise close back over the pair it has
        // already joined, which is the same link a second time.
        if plan.topology == Topology::Ring && plan.switches == 2 {
            links.switches = 1;
        }
        links
    }

    fn advance(&mut self) {
        match self.topology {
            Topology::Mesh => {
                self.b += 1;
                if self.b >= self.switches {
                    self.a += 1;
                    self.b = self.a + 1;
                }
                if self.b >= self.switches {
                    self.a = self.switches;
                }
            }
            // Each switch to the next, and the last back to the first.
            Topology::Ring => {
                self.a += 1;
                self.b = (self.a + 1) % self.switches;
            }
            // Switch 1 is the hub; every other switch hangs off it.
            Topology::Star => {
                self.b += 1;
                if self.b >= self.switches {
                    self.a = self.switches;
                }
            }
        }
    }
}

/// The patch schedule: every link, with the port each end lands in.
pub struct Schedule {
    links: Links,
    /// Lanes to a port, per switch, so a star's hub can differ from its
    /// spokes.
    lanes: Vec<u64>,
    /// Link ends already assigned on each switch, which is what makes the
    /// next port number.
    used: Vec<u64>,
}

impl Schedule {
    pub fn new(plan: &Plan) -> Schedule {
        let lanes = (0..plan.switches)
            .map(|i| match plan.arrangement {
                Arrangement::Straight => 1,
                Arrangement::SplitBothEnds => plan.lanes,
                Arrangement::SplitAtHub if i == 0 => plan.lanes,
                Arrangement::SplitAtHub => 1,
            })
            .collect();
        Schedule {
            links: Links::new(plan),
            lanes,
            used: vec![0; plan.switches as usize],
        }
    }

    fn end(&mut self, switch: u64) -> End {
        let i = switch as usize;
        let lanes = self.lanes[i];
        let n = self.used[i];
        self.used[i] += 1;
        End {
            switch: switch + 1,
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
    use crate::fabric::Media;
    use crate::fabric::ops;
    use crate::fabric::plan::{self, Problem};

    fn plan_of(switches: u64, args: &[&str]) -> Plan {
        let ops: Vec<_> = args.iter().map(|a| ops::parse(a).unwrap()).collect();
        plan::build(switches, &ops, Media::Optic)
            .map_err(|e: Problem| e.to_string())
            .expect("plans")
            .plan
    }

    fn lines(switches: u64, args: &[&str]) -> Vec<String> {
        Schedule::new(&plan_of(switches, args))
            .map(|p| format!("{} {}", p.a, p.b))
            .collect()
    }

    #[test]
    fn a_mesh_lists_every_pair_once() {
        assert_eq!(
            lines(4, &[]),
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
            lines(4, &["/ring"]),
            ["sw1:1 sw2:1", "sw2:2 sw3:1", "sw3:2 sw4:1", "sw4:2 sw1:2"]
        );
        assert_eq!(
            lines(4, &["/star"]),
            ["sw1:1 sw2:1", "sw1:2 sw3:1", "sw1:3 sw4:1"]
        );
        // Two switches in a ring are one link, not the same link twice.
        assert_eq!(lines(2, &["/ring"]), ["sw1:1 sw2:1"]);
    }

    #[test]
    fn lanes_fill_a_port_before_the_next_one_is_used() {
        assert_eq!(
            lines(6, &["@100G", "%400G"]),
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
            lines(6, &["@100G", "%400G", "/star"]),
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
    fn parallel_links_are_listed_one_after_another() {
        assert_eq!(
            lines(3, &["x2"]),
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
        for args in [
            vec![],
            vec!["/ring"],
            vec!["/star"],
            vec!["x2"],
            vec!["/ring", "x3"],
            vec!["@100G", "%4"],
            vec!["@100G", "%4", "/star"],
            vec!["@100G", "%8", "/ring"],
        ] {
            for switches in 2..14u64 {
                let plan = plan_of(switches, &args);
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
                assert_eq!(links, plan.links, "{switches} {args:?}: link count");
                for side in &plan.sides {
                    // The hub is switch 1; everything else is a spoke, and in
                    // the shapes with one kind of switch they are all alike.
                    let range: Vec<usize> = match side.role {
                        plan::Role::Hub => vec![0],
                        plan::Role::Spoke => (1..switches as usize).collect(),
                        plan::Role::Every => (0..switches as usize).collect(),
                    };
                    for i in range {
                        assert_eq!(ends[i], side.degree, "{switches} {args:?}: ends on sw{i}");
                        assert_eq!(ports[i], side.ports, "{switches} {args:?}: ports on sw{i}");
                    }
                }
            }
        }
    }

    /// Nothing is plugged into a port twice: a lane carries one link end.
    #[test]
    fn every_lane_is_used_once() {
        for args in [vec![], vec!["@100G", "%4"], vec!["/ring", "x2"]] {
            for switches in 2..12u64 {
                let plan = plan_of(switches, &args);
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
        let plan = plan_of(4096, &[]);
        assert_eq!(plan.links, 8_386_560);
        let first: Vec<String> = Schedule::new(&plan)
            .take(2)
            .map(|p| format!("{} {}", p.a, p.b))
            .collect();
        assert_eq!(first, ["sw1:1 sw2:1", "sw1:2 sw3:1"]);
    }
}
