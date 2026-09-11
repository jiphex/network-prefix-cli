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
//! the optics sit at the trunk ports. Where the two ends are different - a
//! star's hub, a leaf-spine's spines - the upstream end breaks out and the
//! downstream end takes a whole port each, which is the arrangement a DAC or
//! AOC splitter is built for and the one a 400G spine port fanned out to four
//! 100G leaves is in.
//!
//! Ports facing servers rather than the fabric are counted too, because they
//! are the other half of the only ratio anyone quotes: what is attached to a
//! switch against what leaves it.

use super::ops::{self, Breakout, Op};
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
    /// The upstream end - a star's hub, a leaf-spine's spines - breaks out,
    /// and each downstream switch takes a whole port, which is what a
    /// splitter cable is shaped like.
    SplitUpstream,
}

pub use super::Role;

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
    /// What one such switch faces away from the fabric, when anything was
    /// said about it.
    pub access: Option<Access>,
}

/// The ports on a switch that face servers rather than the fabric, resolved
/// against whatever they are broken out of.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Access {
    /// Server-facing ends on one such switch.
    pub servers: u64,
    pub speed: Speed,
    /// Physical ports those ends consume.
    pub ports: u64,
    pub port_speed: Speed,
    pub lanes: u64,
    pub spare_lanes: u64,
}

/// What is attached to a switch against what leaves it - the ratio every
/// fabric is quoted at, held as its two sides so the report can show both.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Ratio {
    /// Server-facing capacity, in megabits per second.
    pub down: u64,
    /// Fabric-facing capacity, same units.
    pub up: u64,
}

impl Ratio {
    /// Whether the fabric is the narrower half, which is the case anyone
    /// asking about oversubscription is asking about.
    pub fn blocking(&self) -> bool {
        self.down > self.up
    }
}

impl Side {
    /// The ratio this kind of switch runs at, once there is a link speed to
    /// measure the fabric side with.
    pub fn ratio(&self, link: Option<Speed>) -> Option<Ratio> {
        self.ratio_over(link, self.degree)
    }

    /// The same, against some other number of links - what is left after a
    /// spine has gone, say.
    pub fn ratio_over(&self, link: Option<Speed>, degree: u64) -> Option<Ratio> {
        let access = self.access?;
        let link = link?;
        Some(Ratio {
            down: access.speed.mbps() * access.servers,
            up: link.mbps() * degree,
        })
    }

    /// Every port on one such switch, fabric-facing and server-facing alike.
    pub fn total_ports(&self) -> u64 {
        self.ports + self.access.map_or(0, |a| a.ports)
    }
}

/// One line of the bill of materials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub quantity: u64,
    /// The name machines use, which is what `--quiet` prints.
    pub key: String,
    /// The same thing for people.
    pub label: String,
    /// Where it goes, which is the part that stops a count being ambiguous.
    pub note: String,
}

/// The answer to `=N`, saying whether the plan fits a switch with that many
/// ports.
///
/// A bare `=N` is about the whole front panel. `=N@SPEED` is about the ports
/// at one speed, which is how a real switch is specified - 48 at 25G, four at
/// 100G, two at 200G - and the only way to ask whether a plan fits one.
/// `:leaf` on either says which switches are being asked about, for the
/// fabrics where a speed does not pick them out on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Budget {
    pub ports: u64,
    pub speed: Option<Speed>,
    pub role: Option<Role>,
    /// Only the kinds of switch that use a port of that speed at all. A
    /// question about 400G ports has nothing to say about a leaf that has
    /// none.
    pub sides: Vec<BudgetSide>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetSide {
    pub role: Role,
    /// How many switches hold the role, so a row can say "the leaf" when
    /// there is one of them.
    pub switches: u64,
    /// Every port one such switch gives up, fabric and servers together -
    /// which is what has to fit, whatever the ports are facing.
    pub needed: u64,
    pub port_speed: Option<Speed>,
    /// The fabric-facing and server-facing halves of `needed`, for a switch
    /// whose ports are not all the same speed.
    pub fabric: u64,
    pub servers: Option<(u64, Speed)>,
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
    /// Every switch in the fabric, spines included.
    pub switches: u64,
    /// How the switches divide when they are not all the same job. Zero
    /// spines means they are: a mesh, a ring, or a star of peers.
    pub spines: u64,
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

/// What a leaf is left with when one spine is gone, which is the reason
/// anyone buys the second one.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SpineLoss {
    /// Uplinks on a leaf before the loss.
    pub uplinks: u64,
    /// Uplinks on that leaf after it.
    pub left: u64,
}

impl Plan {
    /// The leaves, when the shape has any.
    pub fn leaves(&self) -> Option<&Side> {
        self.sides.iter().find(|s| s.role == Role::Leaf)
    }

    /// What losing one spine costs, for a fabric that has spines to lose. A
    /// lone spine is not a loss anyone plans around - it is the fabric - so
    /// it says nothing here and is flagged as a caution instead.
    pub fn spine_loss(&self) -> Option<SpineLoss> {
        let leaves = self.leaves()?;
        if self.spines < 2 {
            return None;
        }
        Some(SpineLoss {
            uplinks: leaves.degree,
            left: leaves.degree - self.per_pair,
        })
    }
}

/// The whole of what was asked for: the fabric, plus the questions about it.
#[derive(Clone, Debug)]
pub struct Report {
    pub plan: Plan,
    pub budgets: Vec<Budget>,
    pub schedule: bool,
}

/// What the flags say, as against what the operators do: the shape of the
/// fabric, what its links are made of, and whether a patch schedule was
/// asked for. These are choices from a fixed list rather than numbers, which
/// is what makes them flags.
#[derive(Copy, Clone, Debug, Default)]
pub struct Options {
    pub media: Media,
    pub shape: Option<Topology>,
    pub schedule: bool,
}

pub fn build(switches: u64, ops: &[Op], opts: Options) -> Result<Report, Problem> {
    if switches == 0 {
        return Err(Problem::Input(
            "a fabric needs switches in it: give a count, such as 8".into(),
        ));
    }
    if switches > MAX_SWITCHES {
        return Err(Problem::Input(format!(
            "{switches} switches is past what this will plan ({MAX_SWITCHES} is the limit)"
        )));
    }

    let topology = opts.shape;
    let mut speed = None;
    let mut per_pair = None;
    let mut breakout = None;
    let mut spines = None;
    let mut access = None;
    let mut budgets = Vec::new();
    let schedule = opts.schedule;

    // A scalar given twice is a question with two answers, so it is an error
    // rather than a race between them.
    for op in ops {
        match op {
            Op::Speed(s) => set_once(&mut speed, *s, "a link speed")?,
            Op::Breakout(b) => set_once(&mut breakout, *b, "a breakout")?,
            Op::PerPair(n) => set_once(&mut per_pair, u64::from(*n), "a links-per-pair")?,
            Op::Spines(n) => set_once(&mut spines, u64::from(*n), "a spine count")?,
            Op::Access(a) => set_once(&mut access, *a, "a server port count")?,
            Op::Budget(b) => budgets.push(*b),
        }
    }

    // Saying how many spines there are is what makes it a leaf-spine: there
    // is no other shape the answer fits into, and needing to say /leaf-spine
    // as well would be a second way to say the same thing.
    let topology = match (topology, spines) {
        (Some(t), Some(_)) if t != Topology::LeafSpine => {
            return Err(Problem::Input(format!(
                "a {t} has no spines: drop the +, or ask for --shape=leaf-spine"
            )));
        }
        (Some(t), _) => t,
        (None, Some(_)) => Topology::LeafSpine,
        (None, None) => Topology::default(),
    };
    let spines = match topology {
        // A rack is built with a pair of spines, so that is what a
        // leaf-spine means when nobody says otherwise. One is a single point
        // of failure rather than a smaller fabric, and gets said so.
        Topology::LeafSpine => spines.unwrap_or(2),
        _ => 0,
    };
    // One leaf under a pair of spines is a fabric, so the count that has to
    // reach two is the whole fabric's rather than the one that was typed.
    if switches + spines < 2 {
        return Err(Problem::Input(format!(
            "{switches} switch is not a fabric on its own: there has to be something to \
             connect it to"
        )));
    }
    if switches + spines > MAX_SWITCHES {
        return Err(Problem::Input(format!(
            "{switches} leaves and {spines} spines is past what this will plan \
             ({MAX_SWITCHES} switches is the limit)"
        )));
    }
    let per_pair = per_pair.unwrap_or(1);
    if per_pair > MAX_PER_PAIR {
        return Err(Problem::Input(format!(
            "{per_pair} links between one pair is past what this will plan \
             ({MAX_PER_PAIR} is the limit)"
        )));
    }

    let shape = Shape {
        topology,
        count: switches,
        spines,
        per_pair,
    };
    let plan = plan(shape, speed, breakout, access, opts.media)?;
    let budgets = budgets
        .into_iter()
        .map(|b| budget(&plan, b))
        .collect::<Result<Vec<_>, _>>()?;
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

/// The fabric's shape, before anything is said about speed.
///
/// `count` is what the user gave: switches in a mesh, a ring or a star, and
/// leaves in a leaf-spine, where the spines are on top of it rather than part
/// of it. Spines are numbered first, so switch 1 of a leaf-spine is a spine
/// and switch 1 of everything else is an ordinary member.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Shape {
    topology: Topology,
    count: u64,
    spines: u64,
    per_pair: u64,
}

impl Shape {
    fn total(&self) -> u64 {
        self.count + self.spines
    }

    /// Links in the whole fabric.
    fn links(&self) -> u64 {
        let pairs = match self.topology {
            Topology::Mesh => self.count * (self.count - 1) / 2,
            // Two switches in a ring are a pair with one link, not two: the
            // second one would be the same link coming back the other way.
            Topology::Ring if self.count == 2 => 1,
            Topology::Ring => self.count,
            Topology::Star => self.count - 1,
            Topology::LeafSpine => self.count * self.spines,
        };
        pairs * self.per_pair
    }

    /// Link ends on each switch, in switch order.
    fn degrees(&self) -> Vec<u64> {
        let k = self.per_pair;
        match self.topology {
            Topology::Mesh => vec![(self.count - 1) * k; self.count as usize],
            Topology::Ring if self.count == 2 => vec![k; 2],
            Topology::Ring => vec![2 * k; self.count as usize],
            Topology::Star => {
                let mut d = vec![k; self.count as usize];
                d[0] = (self.count - 1) * k;
                d
            }
            // Every leaf reaches every spine, so a spine holds a link for
            // each leaf and a leaf holds one for each spine.
            Topology::LeafSpine => {
                let mut d = vec![self.count * k; self.spines as usize];
                d.extend(std::iter::repeat_n(self.spines * k, self.count as usize));
                d
            }
        }
    }

    /// Switches a packet crosses at worst, counting the links it traverses.
    fn hops(&self) -> u64 {
        match self.topology {
            Topology::Mesh => 1,
            Topology::Ring => self.count / 2,
            Topology::Star if self.count == 2 => 1,
            Topology::Star => 2,
            // Leaf to spine to leaf, whatever the leaf count.
            Topology::LeafSpine => 2,
        }
    }

    /// Links that have to fail before some switch cannot reach some other.
    fn resilience(&self) -> u64 {
        match self.topology {
            Topology::Mesh => (self.count - 1) * self.per_pair,
            Topology::Ring if self.count == 2 => self.per_pair,
            Topology::Ring => 2 * self.per_pair,
            Topology::Star => self.per_pair,
            // A leaf is cut off when all of its uplinks are.
            Topology::LeafSpine => self.spines * self.per_pair,
        }
    }

    /// Links crossing a cut that leaves half the switches on each side - the
    /// bandwidth available when the traffic is as awkward as it can be.
    fn bisection(&self) -> u64 {
        let k = self.per_pair;
        match self.topology {
            Topology::Mesh => (self.count / 2) * self.count.div_ceil(2) * k,
            Topology::Ring if self.count == 2 => k,
            // A ring has to be cut in two places to be cut at all.
            Topology::Ring => 2 * k,
            // Leave the hub with as many spokes as will fit on its side, and
            // the cut is the spokes on the other side.
            Topology::Star => (self.count / 2) * k,
            // Everything between two halves of the leaves goes up and comes
            // back down, so the narrower half's uplinks are the limit.
            Topology::LeafSpine => (self.count / 2) * self.spines * k,
        }
    }

    /// Who the switches are, and which of them break out.
    fn roles(&self, splits: bool) -> Vec<(Role, u64, u64, bool)> {
        let degrees = self.degrees();
        match self.topology {
            // The spines are the upstream end, so they are the ones that
            // break a port out into lanes.
            Topology::LeafSpine => vec![
                (Role::Spine, self.spines, degrees[0], splits),
                (Role::Leaf, self.count, degrees[self.spines as usize], false),
            ],
            // A star's hub and its spokes are different jobs even in a star
            // of two, where they happen to hold the same number of links.
            // Collapsing them there loses the servers, which hang off spokes,
            // and the answer to a question about the hub.
            Topology::Star => vec![
                (Role::Hub, 1, degrees[0], splits),
                (Role::Spoke, self.count - 1, degrees[1], false),
            ],
            _ => vec![(Role::Every, self.count, degrees[0], splits)],
        }
    }

    /// Which switches have servers hanging off them. The fabric's edge is a
    /// leaf, a spoke, or every switch in a shape that has no edge.
    fn edge(&self) -> Role {
        match self.topology {
            Topology::LeafSpine => Role::Leaf,
            Topology::Star => Role::Spoke,
            _ => Role::Every,
        }
    }
}

fn plan(
    shape: Shape,
    speed: Option<Speed>,
    breakout: Option<Breakout>,
    access: Option<ops::Access>,
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

    let topology = shape.topology;
    let arrangement = match (lanes, topology) {
        (1, _) => Arrangement::Straight,
        // Where the two ends of a link are different switches doing different
        // jobs, only the upstream one breaks out; the other takes a port.
        (_, t) if !t.is_uniform() => Arrangement::SplitUpstream,
        (_, _) => Arrangement::SplitBothEnds,
    };

    // A splitter's lanes end in modules, and a module needs a port to go in.
    // Where every switch is identical there is no port running at the lane
    // speed for it to plug into, so the lanes have to meet each other, and only
    // fibre does that.
    if arrangement == Arrangement::SplitBothEnds && !media.has_transceivers() {
        return Err(Problem::Impossible(format!(
            "a {}-lane {} splitter ends in modules, and in a {} every switch has the same \
             ports, so there is nothing at {} for those modules to plug into. Use \
             --media=optic and join the lanes in a patch field, or put the splitter at one \
             end only with --shape=star or --shape=leaf-spine",
            lanes,
            media,
            topology,
            speed.map_or("the link speed".to_string(), |s| s.to_string()),
        )));
    }

    let links = shape.links();
    let sides = sides(&shape, arrangement, lanes, speed, port_speed, access)?;

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
    if topology == Topology::Ring && shape.count == 2 {
        cautions.push(
            "a ring of two switches is a pair: there is one link between them, not two".into(),
        );
    }
    if topology == Topology::Star && shape.count > 2 {
        cautions.push(
            "the hub is a single point of failure: every spoke is cut off when it goes".into(),
        );
    }
    if topology == Topology::LeafSpine && shape.spines == 1 {
        cautions.push("one spine is a single point of failure; a pair is the usual build".into());
    }
    if access.is_some() && speed.is_none() {
        cautions.push(
            "no link speed was given, so there is nothing to measure the servers against: \
             add @100G for a ratio"
                .into(),
        );
    }

    let materials = materials(
        arrangement,
        topology,
        media,
        links,
        trunk_ports,
        lanes,
        speed,
        port_speed,
        &sides,
    );

    Ok(Plan {
        switches: shape.total(),
        spines: shape.spines,
        topology,
        per_pair: shape.per_pair,
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
        hops: shape.hops(),
        resilience: shape.resilience(),
        bisection: shape.bisection(),
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

/// Group the switches by the shape of what they have to hold, and hang the
/// servers off whichever of them is the fabric's edge.
fn sides(
    shape: &Shape,
    arrangement: Arrangement,
    lanes: u64,
    speed: Option<Speed>,
    port_speed: Option<Speed>,
    access: Option<ops::Access>,
) -> Result<Vec<Side>, Problem> {
    let splits = arrangement != Arrangement::Straight;
    let edge = shape.edge();
    let mut sides = Vec::new();
    for (role, switches, degree, breaks_out) in shape.roles(splits) {
        let lanes = if breaks_out { lanes } else { 1 };
        let ports = degree.div_ceil(lanes);
        sides.push(Side {
            role,
            switches,
            degree,
            ports,
            port_speed: if breaks_out { port_speed } else { speed },
            lanes,
            spare_lanes: ports * lanes - degree,
            access: match access {
                Some(a) if role == edge => Some(resolve_access(a)?),
                _ => None,
            },
        });
    }
    Ok(sides)
}

/// Work out what the server-facing ports cost in physical ports, which is the
/// same lane arithmetic the fabric side does - a 100G port is four 25G
/// servers exactly as a 400G port is four 100G leaves.
fn resolve_access(a: ops::Access) -> Result<Access, Problem> {
    let servers = u64::from(a.ports);
    let lanes = lanes(a.breakout, Some(a.speed))?;
    let port_speed = a.speed.times(lanes).ok_or_else(|| {
        Problem::Input(format!(
            "{} split {lanes} ways is not a port speed anything has",
            a.speed
        ))
    })?;
    let ports = servers.div_ceil(lanes);
    Ok(Access {
        servers,
        speed: a.speed,
        ports,
        port_speed,
        lanes,
        spare_lanes: ports * lanes - servers,
    })
}

#[allow(clippy::too_many_arguments)]
fn materials(
    arrangement: Arrangement,
    topology: Topology,
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
    // The two ends of a split have different names depending on the shape,
    // and a note that calls a spine a hub is a note nobody trusts.
    let (up, down) = match topology {
        Topology::LeafSpine => ("spine", "leaf"),
        _ => ("hub", "spoke"),
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
        (Arrangement::SplitUpstream, Media::Optic) => {
            items.push(Item {
                quantity: trunk_ports,
                key: keyed("transceiver", port_speed),
                label: format!("{}transceiver", at(port_speed)),
                note: format!("one in each {up} port"),
            });
            items.push(Item {
                quantity: trunk_ports,
                key: keyed(&format!("breakout-1x{lanes}"), port_speed),
                label: format!("{}breakout harness", harness(true)),
                note: format!("one per {up} port, a lane to each {down}"),
            });
            items.push(Item {
                quantity: links,
                key: keyed("transceiver", speed),
                label: format!("{}transceiver", at(speed)),
                note: format!("one in each {down} port"),
            });
        }
        (Arrangement::SplitUpstream, _) => items.push(Item {
            quantity: trunk_ports,
            key: keyed(&format!("splitter-1x{lanes}"), port_speed),
            label: format!("{}{} splitter", harness(true), media),
            note: format!(
                "one per {up} port, a lane to each of {}",
                plural(lanes, down)
            ),
        }),
    }
    // Ports are not something anyone buys, but they are the thing that runs
    // out, so the count belongs with the rest of the order. Server-facing
    // ports are counted the same way and kept apart by their own key: what
    // hangs off them is bought with the servers, but the ports themselves are
    // spent here.
    for side in sides {
        items.push(Item {
            quantity: side.switches * side.ports,
            key: keyed(&format!("port-{}", side.role.key()), side.port_speed),
            label: format!("{}{} port", at(side.port_speed), side.role.singular()),
            note: format!("{} on {}", plural(side.ports, "port"), whose(side)),
        });
        if let Some(a) = side.access {
            items.push(Item {
                quantity: side.switches * a.ports,
                key: keyed(
                    &format!("access-port-{}", side.role.key()),
                    Some(a.port_speed),
                ),
                label: format!("{} {} server port", a.port_speed, side.role.singular()),
                note: format!(
                    "{} facing {} on {}",
                    plural(a.ports, "port"),
                    plural(a.servers, "server"),
                    whose(side)
                ),
            });
        }
    }
    items
}

/// Whose ports these are, said the way the note reads best.
fn whose(side: &Side) -> String {
    let many = |what: &str| match side.switches {
        1 => format!("the {what}"),
        n => format!("each of {}", plural(n, what)),
    };
    match side.role {
        Role::Every => many("switch"),
        Role::Hub => "the hub".to_string(),
        Role::Spoke => many("spoke"),
        Role::Spine => many("spine"),
        Role::Leaf => many("leaf"),
    }
}

/// The plural of a noun on its own, for a sentence that does not count it.
fn plural_of(what: &str) -> String {
    match what {
        "switch" => "switches".to_string(),
        "leaf" => "leaves".to_string(),
        w => format!("{w}s"),
    }
}

fn plural(n: u64, what: &str) -> String {
    let plural = match what {
        "switch" => "switches".to_string(),
        "leaf" => "leaves".to_string(),
        w => format!("{w}s"),
    };
    format!("{n} {}", if n == 1 { what.to_string() } else { plural })
}

fn budget(plan: &Plan, asked: ops::Budget) -> Result<Budget, Problem> {
    let ports = u64::from(asked.ports);
    // A question about a kind of switch the fabric does not have is a
    // misunderstanding of the fabric rather than a plan that does not fit, so
    // it is refused rather than answered yes.
    if let Some(role) = asked.role
        && !plan.sides.iter().any(|s| s.role == role)
    {
        return Err(Problem::Input(format!(
            "a {} has no {}: it has {}",
            plan.topology,
            plural_of(role.singular()),
            english_list(
                &plan
                    .sides
                    .iter()
                    .map(|s| plural(s.switches, s.role.singular()))
                    .collect::<Vec<_>>()
            )
        )));
    }
    let sides = plan
        .sides
        .iter()
        .filter(|s| asked.role.is_none_or(|role| s.role == role))
        .filter_map(|s| {
            // Servers and the fabric come out of the same front panel, so
            // what has to fit is both of them together - and when the
            // question names a speed, only the ports running at it.
            let (fabric, servers) = match asked.speed {
                None => (s.ports, s.access.map(|a| (a.ports, a.port_speed))),
                Some(at) => (
                    if s.port_speed == Some(at) { s.ports } else { 0 },
                    s.access
                        .filter(|a| a.port_speed == at)
                        .map(|a| (a.ports, a.port_speed)),
                ),
            };
            let needed = fabric + servers.map_or(0, |(n, _)| n);
            if needed == 0 {
                return None;
            }
            Some(BudgetSide {
                role: s.role,
                switches: s.switches,
                needed,
                port_speed: asked.speed.or(s.port_speed),
                fabric,
                servers,
                spare: ports.saturating_sub(needed),
                short: needed.saturating_sub(ports),
            })
        })
        .collect();
    Ok(Budget {
        ports,
        speed: asked.speed,
        role: asked.role,
        sides,
    })
}

/// "16 leaves and 2 spines", for an error that has to say what a fabric does
/// have after saying what it does not.
fn english_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
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

    /// The tests write the shape inline, as `/star`, because a list of cases
    /// reads better that way than one with a separate argument threaded
    /// through it.
    fn options<'a>(args: &[&'a str], media: Media) -> (Vec<&'a str>, Options) {
        let mut opts = Options {
            media,
            ..Options::default()
        };
        let rest = args
            .iter()
            .filter(|a| match a.strip_prefix('/') {
                Some(shape) => {
                    opts.shape = Some(
                        <Topology as clap::ValueEnum>::from_str(&shape.to_ascii_lowercase(), true)
                            .expect("a shape"),
                    );
                    false
                }
                None => true,
            })
            .copied()
            .collect();
        (rest, opts)
    }

    /// `build` as the tests want it, with the shape written inline.
    fn built(switches: u64, args: &[&str], media: Media) -> Result<Report, Problem> {
        let (rest, opts) = options(args, media);
        build(switches, &ops(&rest), opts)
    }

    fn plan_of(switches: u64, args: &[&str]) -> Plan {
        let (rest, opts) = options(args, Media::Optic);
        build(switches, &ops(&rest), opts).expect("plans").plan
    }

    fn fails(switches: u64, args: &[&str], media: Media) -> Problem {
        let (rest, opts) = options(args, media);
        build(switches, &ops(&rest), opts).expect_err("should not plan")
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
        assert!(e.to_string().contains("--shape=star"));
        // The same splitter into a star's spokes is exactly what it is for.
        let p = built(8, &["@100G", "%400G", "/star"], Media::Dac)
            .expect("plans")
            .plan;
        assert_eq!(p.arrangement, Arrangement::SplitUpstream);
        assert_eq!(p.trunk_ports, 2);
    }

    #[test]
    fn straight_cables_do_not_care_about_media() {
        for media in [Media::Optic, Media::Aoc, Media::Dac] {
            let p = built(8, &["@100G"], media).expect("plans").plan;
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
    fn a_leaf_spine_is_every_leaf_to_every_spine() {
        let p = plan_of(16, &["+2"]);
        assert_eq!(p.topology, Topology::LeafSpine);
        assert_eq!(p.switches, 18);
        assert_eq!(p.spines, 2);
        assert_eq!(p.links, 32);
        assert_eq!((p.sides[0].role, p.sides[0].degree), (Role::Spine, 16));
        assert_eq!((p.sides[1].role, p.sides[1].degree), (Role::Leaf, 2));
        assert_eq!(p.sides[1].switches, 16);
        // Two hops, and a leaf is cut off only when all its uplinks are.
        assert_eq!((p.hops, p.resilience), (2, 2));
        // Half the leaves' uplinks are what crosses the middle.
        assert_eq!(p.bisection, 16);
    }

    #[test]
    fn saying_how_many_spines_is_what_makes_it_a_leaf_spine() {
        assert_eq!(plan_of(8, &["+2"]).topology, Topology::LeafSpine);
        // A rack is built with a pair, so that is what the shape means when
        // nobody says otherwise.
        assert_eq!(plan_of(8, &["/leaf-spine"]).spines, 2);
        assert_eq!(plan_of(8, &["/leaf-spine", "+4"]).spines, 4);
        // A shape that has no spines cannot be given any.
        let e = fails(8, &["/mesh", "+2"], Media::Optic);
        assert!(matches!(e, Problem::Input(_)), "{e:?}");
        assert!(e.to_string().contains("no spines"), "{e}");
    }

    #[test]
    fn a_pair_of_spines_is_what_a_leaf_survives_losing_one_of() {
        let p = plan_of(16, &["+2", "@100G", "-48@25G"]);
        let loss = p.spine_loss().expect("a spine to lose");
        assert_eq!((loss.uplinks, loss.left), (2, 1));
        // Losing one halves the fabric a leaf has, so the ratio doubles.
        let degraded = p
            .leaves()
            .unwrap()
            .ratio_over(p.speed, loss.left)
            .expect("a ratio");
        assert_eq!((degraded.down, degraded.up), (1_200_000, 100_000));

        // Two links to each of two spines is four uplinks, and a spine
        // taking two of them with it.
        let p = plan_of(16, &["+2", "x2", "@100G"]);
        let loss = p.spine_loss().unwrap();
        assert_eq!((loss.uplinks, loss.left), (4, 2));

        // One spine is not a loss anyone plans around; it is the fabric.
        let p = plan_of(16, &["+1", "@100G"]);
        assert_eq!(p.spine_loss(), None);
        assert!(
            p.cautions
                .iter()
                .any(|c| c.contains("single point of failure")),
            "{:?}",
            p.cautions
        );
        // And a shape with no spines has none to lose.
        assert_eq!(plan_of(16, &["@100G"]).spine_loss(), None);
    }

    /// The case a rack is actually built as: two leaves, a pair of spines,
    /// and every leaf on every spine.
    #[test]
    fn a_pair_of_leaves_reaches_every_spine() {
        for (leaves, spines) in [(2u64, 2u64), (2, 4), (4, 2), (8, 2)] {
            let p = plan_of(leaves, &[&format!("+{spines}"), "@100G"]);
            assert_eq!(p.links, leaves * spines, "{leaves} leaves, {spines} spines");
            let leaf = p.leaves().expect("leaves");
            assert_eq!(leaf.degree, spines, "a leaf reaches every spine");
            assert_eq!(p.sides[0].degree, leaves, "a spine reaches every leaf");
            assert_eq!(p.switches, leaves + spines);
        }
    }

    #[test]
    fn only_the_spines_break_out() {
        let p = plan_of(16, &["+2", "@100G", "%400G"]);
        assert_eq!(p.arrangement, Arrangement::SplitUpstream);
        // Sixteen leaves over four lanes to a port is four spine ports.
        assert_eq!((p.sides[0].ports, p.sides[0].lanes), (4, 4));
        // And the leaf takes a whole 100G port for each spine.
        assert_eq!((p.sides[1].ports, p.sides[1].lanes), (2, 1));
        assert_eq!(p.sides[1].port_speed, Some(speed::parse("100G").unwrap()));
        assert_eq!(p.trunk_ports, 8);
        // A splitter is fine here: its lanes plug into whole leaf ports.
        assert!(built(16, &["+2", "@100G", "%400G"], Media::Dac).is_ok());
    }

    /// A star of two has a hub and a spoke that hold the same number of
    /// links, and they are still different jobs. Treating them as one
    /// population lost the servers, which hang off spokes, and the answer to
    /// any question about the hub.
    #[test]
    fn a_star_of_two_still_has_a_hub_and_a_spoke() {
        let p = plan_of(2, &["/star", "@100G", "-24@10G"]);
        assert_eq!(p.sides.len(), 2);
        assert_eq!((p.sides[0].role, p.sides[0].switches), (Role::Hub, 1));
        assert_eq!((p.sides[1].role, p.sides[1].switches), (Role::Spoke, 1));
        // The servers are on the spoke, and the ratio is about them.
        assert!(p.sides[0].access.is_none());
        let spoke = &p.sides[1];
        assert_eq!(spoke.access.expect("servers").servers, 24);
        assert_eq!(spoke.ratio(p.speed).expect("a ratio").down, 240_000);
        // And a question about the hub is answerable.
        assert!(built(2, &["/star", "@100G", "=32:hub"], Media::Optic).is_ok());
    }

    #[test]
    fn a_single_leaf_under_spines_is_a_fabric() {
        let p = plan_of(1, &["+2", "@100G"]);
        assert_eq!((p.switches, p.spines, p.links), (3, 2, 2));
        assert_eq!(p.leaves().expect("a leaf").switches, 1);
        // One switch on its own is still not a fabric, and neither is none.
        for (count, args) in [(1u64, vec!["@100G"]), (0, vec!["+2"])] {
            let e = fails(count, &args, Media::Optic);
            assert!(matches!(e, Problem::Input(_)), "{count} {args:?}: {e:?}");
        }
    }

    #[test]
    fn a_hub_is_flagged_the_way_a_lone_spine_is() {
        let p = plan_of(8, &["/star", "@100G"]);
        assert!(
            p.cautions
                .iter()
                .any(|c| c.contains("single point of failure")),
            "{:?}",
            p.cautions
        );
        // A star of two is a pair, where the hub carries no risk a pair does
        // not carry anyway.
        assert!(plan_of(2, &["/star", "@100G"]).cautions.is_empty());
        assert!(plan_of(8, &["@100G"]).cautions.is_empty());
    }

    #[test]
    fn servers_hang_off_whichever_switch_is_the_edge() {
        let edge = |args: &[&str]| {
            plan_of(8, args)
                .sides
                .iter()
                .find(|s| s.access.is_some())
                .map(|s| s.role)
        };
        assert_eq!(edge(&["-48@25G", "+2"]), Some(Role::Leaf));
        assert_eq!(edge(&["-48@25G", "/star"]), Some(Role::Spoke));
        assert_eq!(edge(&["-48@25G"]), Some(Role::Every));
        assert_eq!(edge(&["-48@25G", "/ring"]), Some(Role::Every));
        // A spine has nothing hanging off it; that is what makes it a spine.
        let p = plan_of(8, &["-48@25G", "+2"]);
        assert!(p.sides[0].access.is_none());
    }

    #[test]
    fn a_ratio_is_what_is_attached_against_what_leaves() {
        // 48 x 25G of servers is 1.2T; two 100G uplinks is 200G; 6:1.
        let p = plan_of(16, &["+2", "@100G", "-48@25G"]);
        let leaf = &p.sides[1];
        let r = leaf.ratio(p.speed).expect("a ratio");
        assert_eq!((r.down, r.up), (1_200_000, 200_000));
        assert!(r.blocking());

        // Four spines is four uplinks, and the same servers come out 3:1.
        let p = plan_of(16, &["+4", "@100G", "-48@25G"]);
        let r = p.sides[1].ratio(p.speed).expect("a ratio");
        assert_eq!((r.down, r.up), (1_200_000, 400_000));

        // Enough uplink and it stops being a ratio anyone worries about.
        let p = plan_of(16, &["+12", "@100G", "-48@25G"]);
        assert!(!p.sides[1].ratio(p.speed).unwrap().blocking());

        // With no link speed there is nothing to measure the servers against.
        let p = plan_of(16, &["+2", "-48@25G"]);
        assert_eq!(p.sides[1].ratio(p.speed), None);
        assert!(
            p.cautions.iter().any(|c| c.contains("@100G")),
            "{:?}",
            p.cautions
        );
    }

    #[test]
    fn server_ports_are_broken_out_like_any_others() {
        // 48 servers at 25G out of 100G ports is twelve ports, not 48.
        let p = plan_of(16, &["+2", "@100G", "-48@25G%100G"]);
        let a = p.sides[1].access.expect("servers");
        assert_eq!((a.servers, a.ports, a.lanes), (48, 12, 4));
        assert_eq!(a.port_speed, speed::parse("100G").unwrap());
        assert_eq!(a.spare_lanes, 0);
        // The ratio is about the servers, not the ports they arrive on.
        let r = p.sides[1].ratio(p.speed).unwrap();
        assert_eq!(r.down, 1_200_000);

        // A count that does not divide leaves a part-used port.
        let a = plan_of(16, &["+2", "@100G", "-50@25G%4"]).sides[1]
            .access
            .expect("servers");
        assert_eq!((a.ports, a.spare_lanes), (13, 2));
    }

    /// Servers and the fabric come out of the same front panel, so whatever
    /// is counted against a port budget has to be both of them.
    #[test]
    fn every_port_a_switch_gives_up_is_counted_once() {
        for args in [
            vec!["+2", "@100G", "%400G", "-48@25G"],
            vec!["+4", "@100G", "-48@25G%100G"],
            vec!["/star", "@25G", "-24@10G"],
            vec!["@100G", "-12@25G"],
        ] {
            for switches in 2..12u64 {
                let p = plan_of(switches, &args);
                for side in &p.sides {
                    let access = side.access.map_or(0, |a| a.ports);
                    assert_eq!(
                        side.total_ports(),
                        side.ports + access,
                        "{switches} {args:?} {:?}",
                        side.role
                    );
                }
                let budget = super::budget(
                    &p,
                    ops::Budget {
                        ports: 1_000,
                        speed: None,
                        role: None,
                    },
                )
                .expect("a budget about every switch");
                for (b, side) in budget.sides.iter().zip(&p.sides) {
                    assert_eq!(b.needed, side.total_ports(), "{switches} {args:?}");
                    assert_eq!(b.fabric, side.ports);
                }
            }
        }
    }

    /// A real switch is specified by what it has at each speed - 48 at 25G,
    /// four at 100G, two at 200G - and a single total cannot answer whether a
    /// plan fits one.
    #[test]
    fn a_budget_can_ask_about_one_kind_of_port() {
        let r = built(
            8,
            &[
                "+4", "@100G", "%400G", "-48@25G", "=32@400G", "=4@100G", "=48@25G", "=2@200G",
            ],
            Media::Optic,
        )
        .expect("plans");
        let [spine_400, leaf_100, leaf_25, none_200] = &r.budgets[..] else {
            panic!("four budgets, got {}", r.budgets.len());
        };

        // Two of the spine's 32 400G ports carry all eight leaves.
        assert!(spine_400.fits());
        assert_eq!(spine_400.sides.len(), 1);
        assert_eq!(spine_400.sides[0].role, Role::Spine);
        assert_eq!(
            (spine_400.sides[0].needed, spine_400.sides[0].spare),
            (2, 30)
        );

        // The leaf's four 100G ports are its four uplinks, exactly.
        assert_eq!(leaf_100.sides[0].role, Role::Leaf);
        assert_eq!((leaf_100.sides[0].needed, leaf_100.sides[0].spare), (4, 0));
        assert_eq!(leaf_25.sides[0].needed, 48);

        // Nothing runs at 200G, and that is an answer rather than a fit.
        assert!(none_200.sides.is_empty());
        assert!(none_200.fits());
    }

    /// A speed picks out a kind of switch in most fabrics, but not in one
    /// where two roles have ports at the same speed. Naming the role says
    /// which is being asked about.
    #[test]
    fn a_budget_can_name_the_switches_it_is_about() {
        // Leaves and spines both at 100G: the speed alone cannot separate
        // them, and without a role the question is about both.
        let both = built(8, &["+2", "@100G", "=4@100G"], Media::Optic).expect("plans");
        assert_eq!(both.budgets[0].sides.len(), 2);

        let leaves = built(8, &["+2", "@100G", "=4@100G:leaf"], Media::Optic).expect("plans");
        assert_eq!(leaves.budgets[0].sides.len(), 1);
        assert_eq!(leaves.budgets[0].sides[0].role, Role::Leaf);
        assert_eq!(leaves.budgets[0].sides[0].needed, 2);
        assert!(leaves.budgets[0].fits());

        // The spines of that same fabric need a port per leaf, and eight
        // does not fit in four.
        let spines = built(8, &["+2", "@100G", "=4@100G:spine"], Media::Optic).expect("plans");
        assert_eq!(spines.budgets[0].sides[0].role, Role::Spine);
        assert_eq!(
            (
                spines.budgets[0].sides[0].needed,
                spines.budgets[0].sides[0].short
            ),
            (8, 4)
        );
        assert!(!spines.budgets[0].fits());

        // A role with no ports at that speed is answered, not counted as a
        // fit by default.
        let none = built(8, &["+2", "@100G", "=4@400G:leaf"], Media::Optic).expect("plans");
        assert!(none.budgets[0].sides.is_empty());
    }

    #[test]
    fn a_kind_of_switch_the_fabric_has_not_got_is_refused() {
        for (args, missing) in [
            (vec!["@100G", "=32:leaf"], "leaves"),
            (vec!["@100G", "=32:spine"], "spines"),
            (vec!["/star", "@100G", "=32:spine"], "spines"),
            (vec!["+2", "@100G", "=32:hub"], "hubs"),
        ] {
            let e = fails(8, &args, Media::Optic);
            assert!(matches!(e, Problem::Input(_)), "{args:?}: {e:?}");
            assert!(e.to_string().contains(missing), "{args:?}: {e}");
            // And says what it does have, so the next try is an informed one.
            assert!(e.to_string().contains("it has"), "{args:?}: {e}");
        }
        // The shapes where every switch is alike answer to `switch`.
        assert!(built(8, &["@100G", "=32:switch"], Media::Optic).is_ok());
    }

    #[test]
    fn a_port_speed_the_plan_cannot_reach_is_reported_short() {
        // Four spines at 200G needs four 200G ports on a leaf that has two.
        let r = built(
            8,
            &["+4", "@200G", "%400G", "-48@25G", "=2@200G"],
            Media::Optic,
        )
        .expect("plans");
        let b = &r.budgets[0];
        assert!(!b.fits());
        assert_eq!(b.sides[0].role, Role::Leaf);
        assert_eq!((b.sides[0].needed, b.sides[0].short), (4, 2));
    }

    #[test]
    fn a_budget_is_checked_against_every_kind_of_switch() {
        let r = built(8, &["@100G", "%400G", "=32"], Media::Optic).unwrap();
        assert!(r.budgets[0].fits());
        assert_eq!(r.budgets[0].sides[0].spare, 30);

        // A star's hub runs out long before its spokes do.
        let r = built(48, &["@100G", "/star", "=32"], Media::Optic).unwrap();
        assert!(!r.budgets[0].fits());
        assert_eq!(r.budgets[0].sides[0].short, 15);
        assert_eq!(r.budgets[0].sides[1].short, 0);
    }

    #[test]
    fn a_fabric_needs_at_least_two_switches() {
        for n in [0, 1] {
            assert!(matches!(
                built(n, &[], Media::Optic),
                Err(Problem::Input(_))
            ));
        }
        assert!(built(2, &[], Media::Optic).is_ok());
    }

    #[test]
    fn absurd_sizes_are_refused_rather_than_attempted() {
        assert!(matches!(
            built(MAX_SWITCHES + 1, &[], Media::Optic),
            Err(Problem::Input(_))
        ));
        assert!(matches!(
            built(8, &["x65"], Media::Optic),
            Err(Problem::Input(_))
        ));
        // The largest fabric this will plan still fits the arithmetic.
        let p = plan_of(MAX_SWITCHES, &[]);
        assert_eq!(p.links, MAX_SWITCHES * (MAX_SWITCHES - 1) / 2);
    }

    #[test]
    fn a_scalar_given_twice_is_an_error_rather_than_a_race() {
        // The shape is not in this list: it is a flag, and a flag given twice
        // is clap's business rather than ours.
        for args in [
            vec!["@100G", "@400G"],
            vec!["%4", "%8"],
            vec!["x2", "x4"],
            vec!["+2", "+4"],
            vec!["-24@10G", "-48@25G"],
        ] {
            assert!(
                matches!(built(8, &args, Media::Optic), Err(Problem::Input(_))),
                "{args:?}"
            );
        }
        // Questions are not scalars: two budgets are two questions.
        let r = built(8, &["=32", "=16"], Media::Optic).unwrap();
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
