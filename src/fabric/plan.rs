//! Turning a switch count and a handful of operators into every number the
//! report shows: links, ports, cables, transceivers and bandwidth.
//!
//! The model is deliberately small, and stating it is half the value:
//!
//! - every switch has ports of one speed, and a link runs at the link speed;
//! - when the link speed is lower than the port speed, the port is broken out
//!   into lanes, and each lane is one link end;
//! - a breakout harness therefore fans one port out to several *different*
//!   peers, which is the entire reason anyone buys one.
//!
//! What falls out of that is the arrangement, and the arrangement is what
//! decides the bill of materials. Where every switch is the same - a mesh or
//! a ring - both ends of a link are lanes, so they meet in a patch field and
//! the optics sit at the trunk ports. Where one switch is different - the hub
//! of a star - the hub breaks out and the spokes take a whole port each,
//! which is the arrangement a DAC or AOC splitter is built for.

use super::ops::{Breakout, Op};
use super::speed::Speed;
use super::{Media, Topology};

/// Beyond this the numbers stop being a plan and start being a research
/// project, and the products of switch counts stop being reviewable by eye.
pub const MAX_SWITCHES: u64 = 4096;

/// A link aggregation runs out of members long before this.
pub const MAX_PER_PAIR: u64 = 64;

/// Something the tool cannot do, split by whose problem it is: input that
/// does not make sense, and input that makes sense but describes a fabric
/// nobody can build. They leave by different exit codes, as they do on the
/// prefix side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// Bad input. Exit 1.
    Input(String),
    /// A coherent request that cannot be built as asked. Exit 3.
    Impossible(String),
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::Input(m) | Problem::Impossible(m) => f.write_str(m),
        }
    }
}

/// Which end of a link a breakout sits at, once the topology and the speeds
/// have had their say.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Arrangement {
    /// Port speed and link speed are the same: one cable, two ports.
    Straight,
    /// Every switch breaks out, so the two lanes of a link meet each other
    /// rather than a port, and the join happens in the patch field.
    SplitBothEnds,
    /// The hub breaks out and each spoke takes a whole port, which is what a
    /// splitter cable is shaped like.
    SplitAtHub,
}

/// What a switch in a given part of the fabric has to hold. A mesh and a ring
/// have one of these; a star has two, because its hub and its spokes are not
/// the same shape.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Role {
    Every,
    Hub,
    Spoke,
}

impl Role {
    pub fn label(&self) -> &'static str {
        match self {
            Role::Every => "each switch",
            Role::Hub => "the hub",
            Role::Spoke => "each spoke",
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Role::Every => "switch",
            Role::Hub => "hub",
            Role::Spoke => "spoke",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    pub role: Role,
    /// How many switches are in this role.
    pub switches: u64,
    /// Link ends on one such switch.
    pub degree: u64,
    /// Physical ports one such switch gives up.
    pub ports: u64,
    pub port_speed: Option<Speed>,
    /// Lanes each of those ports is split into; 1 when it is not.
    pub lanes: u64,
    /// Lanes provisioned on one such switch and not used by a link.
    pub spare_lanes: u64,
}

/// One line of the bill of materials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub quantity: u64,
    /// A stable name for machines: this is what `--quiet` prints.
    pub key: String,
    /// The same thing for people.
    pub label: String,
    /// Where it goes, which is the part that stops a count being ambiguous.
    pub note: String,
}

/// The answer to `=N`: whether the plan fits a switch with that many ports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Budget {
    pub ports: u64,
    pub sides: Vec<BudgetSide>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetSide {
    pub role: Role,
    pub needed: u64,
    pub port_speed: Option<Speed>,
    pub spare: u64,
    pub short: u64,
}

impl Budget {
    pub fn fits(&self) -> bool {
        self.sides.iter().all(|s| s.short == 0)
    }
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub switches: u64,
    pub topology: Topology,
    pub per_pair: u64,
    pub speed: Option<Speed>,
    pub media: Media,
    pub arrangement: Arrangement,
    /// Lanes to a port; 1 when nothing is broken out.
    pub lanes: u64,
    pub port_speed: Option<Speed>,
    pub links: u64,
    pub sides: Vec<Side>,
    /// Broken-out ports across the whole fabric.
    pub trunk_ports: u64,
    /// Lanes across the whole fabric that no link uses.
    pub spare_lanes: u64,
    pub materials: Vec<Item>,
    /// Switches a packet crosses, at worst, to get anywhere.
    pub hops: u64,
    /// Links that have to fail before the fabric is in two pieces.
    pub resilience: u64,
    /// Links crossing the middle, when the fabric is cut into two halves.
    pub bisection: u64,
    /// Things that are worth a second look but do not stop the plan.
    pub cautions: Vec<String>,
}

/// The whole of what was asked for: the fabric, plus the questions about it.
#[derive(Clone, Debug)]
pub struct Report {
    pub plan: Plan,
    pub budgets: Vec<Budget>,
    pub schedule: bool,
}

pub fn build(switches: u64, ops: &[Op], media: Media) -> Result<Report, Problem> {
    if switches < 2 {
        return Err(Problem::Input(format!(
            "{switches} switch{} is not a fabric: there has to be something to connect it to",
            if switches == 1 { "" } else { "es" }
        )));
    }
    if switches > MAX_SWITCHES {
        return Err(Problem::Input(format!(
            "{switches} switches is past what this will plan ({MAX_SWITCHES} is the limit)"
        )));
    }

    let mut topology = None;
    let mut speed = None;
    let mut per_pair = None;
    let mut breakout = None;
    let mut budgets = Vec::new();
    let mut schedule = false;

    // A scalar given twice is a question with two answers, so it is an error
    // rather than a race between them.
    for op in ops {
        match op {
            Op::Speed(s) => set_once(&mut speed, *s, "a link speed")?,
            Op::Breakout(b) => set_once(&mut breakout, *b, "a breakout")?,
            Op::PerPair(n) => set_once(&mut per_pair, u64::from(*n), "a links-per-pair")?,
            Op::Shape(t) => set_once(&mut topology, *t, "a topology")?,
            Op::Budget(n) => budgets.push(u64::from(*n)),
            Op::Schedule => schedule = true,
        }
    }

    let topology = topology.unwrap_or_default();
    let per_pair = per_pair.unwrap_or(1);
    if per_pair > MAX_PER_PAIR {
        return Err(Problem::Input(format!(
            "{per_pair} links between one pair is past what this will plan \
             ({MAX_PER_PAIR} is the limit)"
        )));
    }

    let plan = plan(switches, topology, per_pair, speed, breakout, media)?;
    let budgets = budgets.into_iter().map(|n| budget(&plan, n)).collect();
    Ok(Report {
        plan,
        budgets,
        schedule,
    })
}

fn set_once<T: Copy>(slot: &mut Option<T>, value: T, what: &str) -> Result<(), Problem> {
    if slot.is_some() {
        return Err(Problem::Input(format!("{what} was given twice")));
    }
    *slot = Some(value);
    Ok(())
}

fn plan(
    switches: u64,
    topology: Topology,
    per_pair: u64,
    speed: Option<Speed>,
    breakout: Option<Breakout>,
    media: Media,
) -> Result<Plan, Problem> {
    let mut cautions = Vec::new();
    let lanes = lanes(breakout, speed)?;
    let port_speed = match (speed, lanes) {
        (Some(s), n) => Some(s.times(n).ok_or_else(|| {
            Problem::Input(format!(
                "{s} split {n} ways is not a port speed anything has"
            ))
        })?),
        (None, _) => None,
    };

    let arrangement = match (lanes, topology) {
        (1, _) => Arrangement::Straight,
        (_, Topology::Star) => Arrangement::SplitAtHub,
        (_, _) => Arrangement::SplitBothEnds,
    };

    // A splitter's lanes end in modules, and a module needs a port to go in.
    // Where every switch is identical there is no port running at the lane
    // speed for it to land in, so the lanes have to meet each other, and only
    // fibre does that.
    if arrangement == Arrangement::SplitBothEnds && !media.has_transceivers() {
        return Err(Problem::Impossible(format!(
            "a {}-lane {} splitter ends in modules, and in a {} every switch has the same \
             ports, so there is nothing at {} for those modules to plug into. Use \
             --media=optic and join the lanes in a patch field, or put the splitter at one \
             end only with /star",
            lanes,
            media,
            topology,
            speed.map_or("the link speed".to_string(), |s| s.to_string()),
        )));
    }

    let links = links(switches, topology, per_pair);
    let degrees = degrees(switches, topology, per_pair);
    let sides = sides(&degrees, arrangement, lanes, speed, port_speed);

    let trunk_ports = sides
        .iter()
        .filter(|s| s.lanes > 1)
        .map(|s| s.switches * s.ports)
        .sum();
    let spare_lanes = sides
        .iter()
        .map(|s| s.switches * s.spare_lanes)
        .sum::<u64>();

    if let Some(s) = speed
        && !s.is_standard()
    {
        cautions.push(format!("{s} is not a standard Ethernet rate"));
    }
    if let Some(p) = port_speed
        && lanes > 1
        && !p.is_standard()
    {
        cautions.push(format!(
            "{p} is not a standard Ethernet rate, so nothing has a port at it"
        ));
    }
    if lanes > 1 && !lanes.is_power_of_two() {
        cautions.push(format!(
            "a {lanes}-lane breakout is not a shape optics come in; 2, 4 and 8 are"
        ));
    }
    if topology == Topology::Ring && switches == 2 {
        cautions.push(
            "a ring of two switches is a pair: there is one link between them, not two".into(),
        );
    }

    let materials = materials(
        arrangement,
        media,
        links,
        trunk_ports,
        lanes,
        speed,
        port_speed,
        &sides,
    );

    Ok(Plan {
        switches,
        topology,
        per_pair,
        speed,
        media,
        arrangement,
        lanes,
        port_speed,
        links,
        sides,
        trunk_ports,
        spare_lanes,
        materials,
        hops: hops(switches, topology),
        resilience: resilience(switches, topology, per_pair),
        bisection: bisection(switches, topology, per_pair),
        cautions,
    })
}

/// How many lanes a port is split into, from whichever way the user said it.
fn lanes(breakout: Option<Breakout>, speed: Option<Speed>) -> Result<u64, Problem> {
    match breakout {
        None => Ok(1),
        Some(Breakout::Lanes(n)) => Ok(u64::from(n)),
        Some(Breakout::Port(port)) => {
            let Some(link) = speed else {
                return Err(Problem::Input(format!(
                    "%{port} says the ports are {port}, but not what a link is: add @100G, or \
                     say how many lanes with %4"
                )));
            };
            if port < link {
                return Err(Problem::Input(format!(
                    "a {port} port cannot carry a {link} link"
                )));
            }
            match port.lanes_over(link) {
                Some(n) => Ok(u64::from(n)),
                None => Err(Problem::Input(format!(
                    "a {port} port does not divide into {link} lanes"
                ))),
            }
        }
    }
}

/// Links in the whole fabric.
pub fn links(switches: u64, topology: Topology, per_pair: u64) -> u64 {
    let pairs = match topology {
        Topology::Mesh => switches * (switches - 1) / 2,
        // Two switches in a ring are a pair with one link, not two: the
        // second one would be the same link coming back the other way.
        Topology::Ring if switches == 2 => 1,
        Topology::Ring => switches,
        Topology::Star => switches - 1,
    };
    pairs * per_pair
}

/// The number of link ends on each switch, in switch order. The star is the
/// only shape where they differ, and its hub is switch 1.
fn degrees(switches: u64, topology: Topology, per_pair: u64) -> Vec<u64> {
    match topology {
        Topology::Mesh => vec![(switches - 1) * per_pair; switches as usize],
        Topology::Ring if switches == 2 => vec![per_pair; 2],
        Topology::Ring => vec![2 * per_pair; switches as usize],
        Topology::Star => {
            let mut d = vec![per_pair; switches as usize];
            d[0] = (switches - 1) * per_pair;
            d
        }
    }
}

/// Group the switches by the shape of what they have to hold.
fn sides(
    degrees: &[u64],
    arrangement: Arrangement,
    lanes: u64,
    speed: Option<Speed>,
    port_speed: Option<Speed>,
) -> Vec<Side> {
    let side = |role: Role, switches: u64, degree: u64, splits: bool| {
        let lanes = if splits { lanes } else { 1 };
        let ports = degree.div_ceil(lanes);
        Side {
            role,
            switches,
            degree,
            ports,
            port_speed: if splits { port_speed } else { speed },
            lanes,
            spare_lanes: ports * lanes - degree,
        }
    };
    match arrangement {
        // A star's hub is the only switch that breaks out; the spokes each
        // give up a whole port at the link speed, which is what the far end
        // of a splitter cable needs.
        Arrangement::SplitAtHub => vec![
            side(Role::Hub, 1, degrees[0], true),
            side(Role::Spoke, degrees.len() as u64 - 1, degrees[1], false),
        ],
        _ => {
            let splits = arrangement == Arrangement::SplitBothEnds;
            match degrees.iter().all(|d| *d == degrees[0]) {
                true => vec![side(Role::Every, degrees.len() as u64, degrees[0], splits)],
                // A star with straight cables: the hub is still a different
                // shape from its spokes even when nothing is broken out.
                false => vec![
                    side(Role::Hub, 1, degrees[0], splits),
                    side(Role::Spoke, degrees.len() as u64 - 1, degrees[1], splits),
                ],
            }
        }
    }
}

/// Switches a packet crosses at worst, counting the links it traverses.
fn hops(switches: u64, topology: Topology) -> u64 {
    match topology {
        Topology::Mesh => 1,
        Topology::Ring => switches / 2,
        Topology::Star if switches == 2 => 1,
        Topology::Star => 2,
    }
}

/// Links that have to fail before some switch cannot reach some other.
fn resilience(switches: u64, topology: Topology, per_pair: u64) -> u64 {
    match topology {
        Topology::Mesh => (switches - 1) * per_pair,
        Topology::Ring if switches == 2 => per_pair,
        Topology::Ring => 2 * per_pair,
        Topology::Star => per_pair,
    }
}

/// Links crossing a cut that leaves half the switches on each side - the
/// bandwidth available when the traffic is as awkward as it can be.
fn bisection(switches: u64, topology: Topology, per_pair: u64) -> u64 {
    match topology {
        Topology::Mesh => (switches / 2) * switches.div_ceil(2) * per_pair,
        Topology::Ring if switches == 2 => per_pair,
        // A ring has to be cut in two places to be cut at all.
        Topology::Ring => 2 * per_pair,
        // Leave the hub with as many spokes as will fit on its side, and the
        // cut is the spokes on the other side.
        Topology::Star => (switches / 2) * per_pair,
    }
}

#[allow(clippy::too_many_arguments)]
fn materials(
    arrangement: Arrangement,
    media: Media,
    links: u64,
    trunk_ports: u64,
    lanes: u64,
    speed: Option<Speed>,
    port_speed: Option<Speed>,
    sides: &[Side],
) -> Vec<Item> {
    let at = |s: Option<Speed>| s.map_or(String::new(), |s| format!("{s} "));
    let keyed = |base: &str, s: Option<Speed>| match s {
        Some(s) => format!("{base}-{s}"),
        None => base.to_string(),
    };
    let harness = |long: bool| match (port_speed, speed) {
        (Some(p), Some(l)) if long => format!("{p} to {lanes}x{l} "),
        _ => format!("1-to-{lanes} "),
    };
    let mut items = Vec::new();
    match (arrangement, media) {
        (Arrangement::Straight, Media::Optic) => {
            items.push(Item {
                quantity: 2 * links,
                key: keyed("transceiver", speed),
                label: format!("{}transceiver", at(speed)),
                note: "one in each end of every link".into(),
            });
            items.push(Item {
                quantity: links,
                key: "patch-lead".into(),
                label: "duplex fibre patch lead".into(),
                note: "one per link".into(),
            });
        }
        (Arrangement::Straight, _) => items.push(Item {
            quantity: links,
            key: keyed(&media.name().to_lowercase(), speed),
            label: format!("{}{}", at(speed), media),
            note: "one per link, ends attached".into(),
        }),
        (Arrangement::SplitBothEnds, _) => {
            items.push(Item {
                quantity: trunk_ports,
                key: keyed("transceiver", port_speed),
                label: format!("{}transceiver", at(port_speed)),
                note: "one in each trunk port".into(),
            });
            items.push(Item {
                quantity: trunk_ports,
                key: keyed(&format!("breakout-1x{lanes}"), port_speed),
                label: format!("{}breakout harness", harness(true)),
                note: "one per trunk port, its lanes fanned out to that many peers".into(),
            });
            items.push(Item {
                quantity: links,
                key: keyed("coupler", speed),
                label: "duplex coupler".into(),
                note: "one per link, where its two lanes meet in the patch field".into(),
            });
        }
        (Arrangement::SplitAtHub, Media::Optic) => {
            items.push(Item {
                quantity: trunk_ports,
                key: keyed("transceiver", port_speed),
                label: format!("{}transceiver", at(port_speed)),
                note: "one in each hub port".into(),
            });
            items.push(Item {
                quantity: trunk_ports,
                key: keyed(&format!("breakout-1x{lanes}"), port_speed),
                label: format!("{}breakout harness", harness(true)),
                note: "one per hub port, a lane to each spoke".into(),
            });
            items.push(Item {
                quantity: links,
                key: keyed("transceiver", speed),
                label: format!("{}transceiver", at(speed)),
                note: "one in each spoke port".into(),
            });
        }
        (Arrangement::SplitAtHub, _) => items.push(Item {
            quantity: trunk_ports,
            key: keyed(&format!("splitter-1x{lanes}"), port_speed),
            label: format!("{}{} splitter", harness(true), media),
            note: format!("one per hub port, a lane to each of {lanes} spokes"),
        }),
    }
    // Ports are not something anyone buys, but they are the thing that runs
    // out, so the count belongs with the rest of the order.
    for side in sides {
        items.push(Item {
            quantity: side.switches * side.ports,
            key: keyed(&format!("port-{}", side.role.key()), side.port_speed),
            label: format!("{}port", at(side.port_speed)),
            note: format!(
                "{} on {}",
                plural(side.ports, "port"),
                match side.role {
                    Role::Every if side.switches == 1 => "the switch".to_string(),
                    Role::Every => format!("each of {} switches", side.switches),
                    Role::Hub => "the hub".to_string(),
                    Role::Spoke if side.switches == 1 => "the spoke".to_string(),
                    Role::Spoke => format!("each of {} spokes", side.switches),
                }
            ),
        });
    }
    items
}

fn plural(n: u64, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

fn budget(plan: &Plan, ports: u64) -> Budget {
    Budget {
        ports,
        sides: plan
            .sides
            .iter()
            .map(|s| BudgetSide {
                role: s.role,
                needed: s.ports,
                port_speed: s.port_speed,
                spare: ports.saturating_sub(s.ports),
                short: s.ports.saturating_sub(ports),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fabric::speed;

    fn ops(args: &[&str]) -> Vec<Op> {
        args.iter()
            .map(|a| super::super::ops::parse(a).unwrap())
            .collect()
    }

    fn plan_of(switches: u64, args: &[&str]) -> Plan {
        build(switches, &ops(args), Media::Optic)
            .expect("plans")
            .plan
    }

    fn fails(switches: u64, args: &[&str], media: Media) -> Problem {
        build(switches, &ops(args), media).expect_err("should not plan")
    }

    #[test]
    fn a_mesh_is_a_link_for_every_pair() {
        for (n, want) in [(2, 1), (4, 6), (8, 28), (16, 120)] {
            assert_eq!(plan_of(n, &[]).links, want, "{n} switches");
        }
    }

    #[test]
    fn parallel_links_multiply_every_pair() {
        let p = plan_of(8, &["x2"]);
        assert_eq!(p.links, 56);
        assert_eq!(p.sides[0].degree, 14);
        assert_eq!(p.sides[0].ports, 14);
    }

    #[test]
    fn a_ring_is_a_link_per_switch_and_a_pair_is_the_exception() {
        assert_eq!(plan_of(8, &["/ring"]).links, 8);
        assert_eq!(plan_of(3, &["/ring"]).links, 3);
        assert_eq!(plan_of(2, &["/ring"]).links, 1);
        assert_eq!(plan_of(8, &["/ring"]).sides[0].degree, 2);
    }

    #[test]
    fn a_star_has_two_kinds_of_switch() {
        let p = plan_of(8, &["/star"]);
        assert_eq!(p.links, 7);
        assert_eq!(p.sides.len(), 2);
        assert_eq!((p.sides[0].role, p.sides[0].degree), (Role::Hub, 7));
        assert_eq!((p.sides[1].role, p.sides[1].switches), (Role::Spoke, 7));
        assert_eq!(p.sides[1].degree, 1);
    }

    /// Every link has two ends, and every end sits on exactly one switch.
    /// This is the property that catches a topology whose link count and
    /// degree disagree, which is the easiest thing here to get wrong.
    #[test]
    fn link_ends_and_degrees_always_agree() {
        for topology in ["/mesh", "/ring", "/star"] {
            for switches in 2..24u64 {
                for per_pair in ["x1", "x2", "x3"] {
                    let p = plan_of(switches, &[topology, per_pair]);
                    let ends: u64 = p.sides.iter().map(|s| s.switches * s.degree).sum();
                    assert_eq!(
                        ends,
                        2 * p.links,
                        "{topology} of {switches} {per_pair}: {ends} ends for {} links",
                        p.links
                    );
                    let counted: u64 = p.sides.iter().map(|s| s.switches).sum();
                    assert_eq!(counted, switches, "{topology} of {switches}");
                }
            }
        }
    }

    /// Lanes are provisioned in whole ports, so every lane is either carrying
    /// a link end or spare - and the two together are the ports times their
    /// lane count. A ports figure that forgets a remainder breaks this.
    #[test]
    fn lanes_are_either_used_or_spare() {
        for switches in 2..20u64 {
            for args in [
                vec!["@100G", "%4"],
                vec!["@100G", "%8"],
                vec!["@100G", "%400G"],
                vec!["@100G", "%4", "x2"],
                vec!["@100G", "%4", "/star"],
                vec!["@100G", "%4", "/ring"],
                vec!["@100G"],
            ] {
                let p = plan_of(switches, &args);
                for side in &p.sides {
                    assert_eq!(
                        side.ports * side.lanes,
                        side.degree + side.spare_lanes,
                        "{switches} switches {args:?} {:?}",
                        side.role
                    );
                }
                let provisioned: u64 = p.sides.iter().map(|s| s.switches * s.ports * s.lanes).sum();
                assert_eq!(provisioned, 2 * p.links + p.spare_lanes, "{args:?}");
            }
        }
    }

    #[test]
    fn a_breakout_is_the_port_speed_over_the_link_speed() {
        let p = plan_of(8, &["@100G", "%400G"]);
        assert_eq!(p.lanes, 4);
        assert_eq!(p.port_speed, Some(speed::parse("400G").unwrap()));
        assert_eq!(p.arrangement, Arrangement::SplitBothEnds);
        // Seven peers over four lanes to a port is two ports, one lane spare.
        assert_eq!(p.sides[0].ports, 2);
        assert_eq!(p.sides[0].spare_lanes, 1);
        assert_eq!(p.trunk_ports, 16);
        assert_eq!(p.spare_lanes, 8);
    }

    #[test]
    fn lanes_can_be_given_without_any_speeds_at_all() {
        let p = plan_of(8, &["%4"]);
        assert_eq!(p.lanes, 4);
        assert_eq!(p.port_speed, None);
        assert_eq!(p.trunk_ports, 16);
    }

    #[test]
    fn a_port_speed_needs_a_link_speed_to_divide_into() {
        let e = fails(8, &["%400G"], Media::Optic);
        assert!(matches!(e, Problem::Input(_)), "{e:?}");
        assert!(e.to_string().contains("@100G"));
        assert!(matches!(
            fails(8, &["@40G", "%100G"], Media::Optic),
            Problem::Input(_)
        ));
        assert!(matches!(
            fails(8, &["@400G", "%100G"], Media::Optic),
            Problem::Input(_)
        ));
    }

    #[test]
    fn a_splitter_needs_somewhere_for_its_modules_to_go() {
        // Both ends of a mesh link are lanes, so a DAC splitter has nothing
        // to plug into. That is a buildability problem, not a typo.
        let e = fails(8, &["@100G", "%400G"], Media::Dac);
        assert!(matches!(e, Problem::Impossible(_)), "{e:?}");
        assert!(e.to_string().contains("/star"));
        // The same splitter into a star's spokes is exactly what it is for.
        let p = build(8, &ops(&["@100G", "%400G", "/star"]), Media::Dac)
            .expect("plans")
            .plan;
        assert_eq!(p.arrangement, Arrangement::SplitAtHub);
        assert_eq!(p.trunk_ports, 2);
    }

    #[test]
    fn straight_cables_do_not_care_about_media() {
        for media in [Media::Optic, Media::Aoc, Media::Dac] {
            let p = build(8, &ops(&["@100G"]), media).expect("plans").plan;
            assert_eq!(p.arrangement, Arrangement::Straight);
            assert_eq!(p.lanes, 1);
            assert_eq!(p.spare_lanes, 0);
        }
    }

    #[test]
    fn the_bill_counts_one_end_per_link_end() {
        let p = plan_of(8, &["@100G"]);
        let optics = p.materials.iter().find(|i| i.key == "transceiver-100G");
        assert_eq!(optics.map(|i| i.quantity), Some(56));
        let leads = p.materials.iter().find(|i| i.key == "patch-lead");
        assert_eq!(leads.map(|i| i.quantity), Some(28));
    }

    #[test]
    fn breaking_out_buys_fewer_optics_than_it_saves_links() {
        let straight = plan_of(8, &["@100G"]);
        let broken = plan_of(8, &["@100G", "%400G"]);
        let count = |p: &Plan, key: &str| {
            p.materials
                .iter()
                .find(|i| i.key == key)
                .map_or(0, |i| i.quantity)
        };
        assert_eq!(count(&straight, "transceiver-100G"), 56);
        assert_eq!(count(&broken, "transceiver-400G"), 16);
        assert_eq!(count(&broken, "breakout-1x4-400G"), 16);
        assert_eq!(count(&broken, "coupler-100G"), 28);
    }

    #[test]
    fn a_star_with_a_splitter_puts_an_optic_at_each_spoke() {
        let p = plan_of(9, &["@25G", "%100G", "/star"]);
        let count = |key: &str| {
            p.materials
                .iter()
                .find(|i| i.key == key)
                .map_or(0, |i| i.quantity)
        };
        assert_eq!(p.links, 8);
        assert_eq!(p.trunk_ports, 2);
        assert_eq!(count("transceiver-100G"), 2);
        assert_eq!(count("breakout-1x4-100G"), 2);
        assert_eq!(count("transceiver-25G"), 8);
    }

    #[test]
    fn bandwidth_counts_are_what_the_shapes_promise() {
        // A mesh of 8 cut down the middle is four switches talking to four.
        assert_eq!(plan_of(8, &[]).bisection, 16);
        assert_eq!(plan_of(8, &["x2"]).bisection, 32);
        // A ring is two links wherever it is cut, however big it is.
        assert_eq!(plan_of(64, &["/ring"]).bisection, 2);
        assert_eq!(plan_of(8, &["/star"]).bisection, 4);
        // And an odd mesh is the two nearest halves.
        assert_eq!(plan_of(7, &[]).bisection, 12);
    }

    #[test]
    fn resilience_and_hops_describe_the_shape() {
        let mesh = plan_of(8, &[]);
        assert_eq!((mesh.hops, mesh.resilience), (1, 7));
        let ring = plan_of(8, &["/ring"]);
        assert_eq!((ring.hops, ring.resilience), (4, 2));
        let star = plan_of(8, &["/star"]);
        assert_eq!((star.hops, star.resilience), (2, 1));
    }

    #[test]
    fn a_budget_is_checked_against_every_kind_of_switch() {
        let r = build(8, &ops(&["@100G", "%400G", "=32"]), Media::Optic).unwrap();
        assert!(r.budgets[0].fits());
        assert_eq!(r.budgets[0].sides[0].spare, 30);

        // A star's hub runs out long before its spokes do.
        let r = build(48, &ops(&["@100G", "/star", "=32"]), Media::Optic).unwrap();
        assert!(!r.budgets[0].fits());
        assert_eq!(r.budgets[0].sides[0].short, 15);
        assert_eq!(r.budgets[0].sides[1].short, 0);
    }

    #[test]
    fn a_fabric_needs_at_least_two_switches() {
        for n in [0, 1] {
            assert!(matches!(
                build(n, &[], Media::Optic),
                Err(Problem::Input(_))
            ));
        }
        assert!(build(2, &[], Media::Optic).is_ok());
    }

    #[test]
    fn absurd_sizes_are_refused_rather_than_attempted() {
        assert!(matches!(
            build(MAX_SWITCHES + 1, &[], Media::Optic),
            Err(Problem::Input(_))
        ));
        assert!(matches!(
            build(8, &ops(&["x65"]), Media::Optic),
            Err(Problem::Input(_))
        ));
        // The largest fabric this will plan still fits the arithmetic.
        let p = plan_of(MAX_SWITCHES, &[]);
        assert_eq!(p.links, MAX_SWITCHES * (MAX_SWITCHES - 1) / 2);
    }

    #[test]
    fn a_scalar_given_twice_is_an_error_rather_than_a_race() {
        for args in [
            vec!["@100G", "@400G"],
            vec!["%4", "%8"],
            vec!["x2", "x4"],
            vec!["/ring", "/star"],
        ] {
            assert!(
                matches!(build(8, &ops(&args), Media::Optic), Err(Problem::Input(_))),
                "{args:?}"
            );
        }
        // Questions are not scalars: two budgets are two questions.
        let r = build(8, &ops(&["=32", "=16"]), Media::Optic).unwrap();
        assert_eq!(r.budgets.len(), 2);
    }

    #[test]
    fn odd_speeds_and_lane_counts_are_flagged_not_refused() {
        let p = plan_of(8, &["@300G"]);
        assert!(p.cautions.iter().any(|c| c.contains("standard")));
        let p = plan_of(8, &["@100G", "%3"]);
        assert_eq!(p.lanes, 3);
        assert!(p.cautions.iter().any(|c| c.contains("3-lane")));
        assert!(plan_of(8, &["@100G"]).cautions.is_empty());
    }
}
