//! The expression that describes a fabric.
//!
//! ```text
//!   8 switches[400G] -100G- mesh
//!   24 switches -100G- ring
//!   1 hub[100G] -25G:dac- 47 spokes
//!   2 spines[400G] -100G- 16 leaves -25G- 48 servers
//!   4 spines[32x400G] -100G- 8 leaves[48x25G,2x200G,4x100G] -25G- 48 servers
//! ```
//!
//! A fabric is a chain of tiers running from the core to the edge, and the
//! thing between two tiers is the link that joins them. That is how the
//! hardware is arranged, so the notation draws it rather than describing it.
//!
//! One argument says what the fabric is and the flags say what to print about
//! it, which is the whole of the rule for where anything goes.
//!
//! Three pieces of syntax carry everything. `N name` is a tier, `[...]` is
//! that switch's front panel as `count x speed` entries, and `-SPEED-` is the
//! links between the tiers either side of it, carrying `KxSPEED` for parallel
//! links and `:dac` for what they are made of.
//!
//! Lanes are never written down. A port carries the link it is given, and
//! where the port is faster than the link it breaks out; the port that
//! carries a link is the smallest one at or above the link's speed. A fabric
//! with no speeds at all breaks nothing out and simply counts cables.

use super::speed::{self, Speed};
use super::{Media, Role, Topology};

/// What the planner is being asked for, which is the same request whichever
/// notation produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// The speed of one link.
    Speed(Speed),
    /// Split each port on the tier that breaks out into lanes.
    Breakout(Breakout),
    /// How many parallel links join each pair of switches.
    PerPair(u32),
    /// How many ports a switch has, as a question the report answers.
    Budget(Budget),
    /// How many spines sit above the leaves.
    Spines(u32),
    /// The ports on each edge switch that face servers rather than the
    /// fabric. They are what an oversubscription ratio is a ratio of.
    Access(Access),
}

/// A port count to check the plan against, and what kind of port it counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub ports: u32,
    /// The speed those ports run at, when the panel entry gave one.
    pub speed: Option<Speed>,
    /// The switches it is about, which the tier it was written on decides.
    pub role: Option<Role>,
}

/// Ports facing whatever hangs off the fabric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    pub ports: u32,
    pub speed: Speed,
    /// The port those servers arrive on, when it is faster than they are.
    pub breakout: Option<Breakout>,
}

/// How a port is broken out. The notation only ever gives a port speed, and
/// how many lanes that is depends on the link speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breakout {
    Lanes(u32),
    Port(Speed),
}

/// The whole request, ready for the planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The count the planner works from: the leaves of a leaf-spine, or every
    /// switch of a flat fabric.
    pub switches: u64,
    pub ops: Vec<Op>,
    pub shape: Topology,
    pub media: Media,
}

/// The most ports one switch ever gives up. A dense leaf has 48 or 64 and a
/// chassis a few hundred, so this leaves room for any of them and still
/// catches a count typed into the wrong place.
const MAX_PORTS: u32 = 4096;

/// One front-panel entry, with either half optional: `[32]` is a count,
/// `[400G]` a speed, and `[32x400G]` both.
type Port = (Option<u32>, Option<Speed>);

/// One tier as it was written, before anything is worked out from it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Tier {
    count: u64,
    name: Name,
    /// The front panel, as `(count, speed)` with either half optional.
    panel: Vec<Port>,
    /// Where in the expression it started, for pointing at it in an error.
    word: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Name {
    Switches,
    Spines,
    Leaves,
    Hub,
    Spokes,
    Servers,
}

impl Name {
    fn parse(word: &str) -> Option<Name> {
        Some(match word {
            "switch" | "switches" => Name::Switches,
            "spine" | "spines" => Name::Spines,
            "leaf" | "leaves" => Name::Leaves,
            "hub" | "hubs" => Name::Hub,
            "spoke" | "spokes" => Name::Spokes,
            "server" | "servers" => Name::Servers,
            _ => return None,
        })
    }

    fn role(self) -> Option<Role> {
        Some(match self {
            Name::Switches => Role::Every,
            Name::Spines => Role::Spine,
            Name::Leaves => Role::Leaf,
            Name::Hub => Role::Hub,
            Name::Spokes => Role::Spoke,
            Name::Servers => return None,
        })
    }
}

/// The link between two tiers, as it was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Link {
    speed: Option<Speed>,
    per_pair: Option<u32>,
    media: Option<Media>,
}

/// Read the whole expression.
pub fn parse(text: &str) -> Result<Request, String> {
    let mut p = Cursor::new(text);
    let first = p.tier()?;
    let link = p.link()?;

    // The far side of a link is either the next tier or, for a fabric with
    // only one tier, the pattern its own switches are wired in.
    let (shape, second) = match p.shape_word() {
        Some(shape) => (Some(shape), None),
        None => (None, Some(p.tier()?)),
    };

    // Servers are the one tier that is not switches, so they are the one the
    // chain may always end with.
    let servers = if p.at_end() {
        None
    } else {
        let link = p.link()?;
        let tier = p.tier()?;
        if tier.name != Name::Servers {
            return Err(third_tier(&tier));
        }
        Some((link, tier))
    };
    p.finish()?;

    assemble(first, link, shape, second, servers)
}

/// A chain of three switch tiers is a super-spine layer, which this does not
/// plan. Saying what it is beats complaining about how many tiers there are.
fn third_tier(tier: &Tier) -> String {
    format!(
        "'{}' is a third tier of switches, which is a super-spine layer and is not planned \
         here. Run it as two fabrics: the spines against the super-spines, then the leaves \
         against the spines",
        tier.word
    )
}

/// Turn what was written into what the planner takes.
fn assemble(
    first: Tier,
    link: Link,
    shape: Option<Topology>,
    second: Option<Tier>,
    servers: Option<(Link, Tier)>,
) -> Result<Request, String> {
    let mut ops = Vec::new();

    // Whichever tier breaks out is the one the optics sit at: the single tier
    // of a flat fabric, or the upstream tier of a tiered one.
    let (shape, switches, upstream, edge) = match (shape, second) {
        (Some(shape), None) => {
            if first.name != Name::Switches {
                return Err(format!(
                    "a {shape} is one tier of switches, so '{}' has nothing above it: write it \
                     as '{} switches'",
                    first.word, first.count
                ));
            }
            (shape, first.count, first.clone(), first)
        }
        (None, Some(second)) => {
            let shape = pair(&first, &second)?;
            let switches = match shape {
                // A star counts its hub among its switches; a leaf-spine
                // counts the leaves and is told the spines separately.
                Topology::Star => first.count + second.count,
                _ => {
                    ops.push(Op::Spines(narrow(first.count, "spines")?));
                    second.count
                }
            };
            (shape, switches, first, second)
        }
        _ => unreachable!("the parser gives exactly one of a shape word and a second tier"),
    };

    if let Some(s) = link.speed {
        ops.push(Op::Speed(s));
    }
    if let Some(k) = link.per_pair {
        ops.push(Op::PerPair(k));
    }
    // The port a link runs on is the smallest one at or above it, and the
    // port being the faster of the two is what breaking out means.
    if let (Some(link_speed), Some(port)) = (link.speed, carrier(&upstream, link.speed)?)
        && port > link_speed
    {
        ops.push(Op::Breakout(Breakout::Port(port)));
    }

    if let Some((server_link, tier)) = servers {
        let Some(speed) = server_link.speed else {
            return Err("the link to the servers needs a speed: write it like -25G-".into());
        };
        let breakout = carrier(&edge, Some(speed))?
            .filter(|port| *port > speed)
            .map(Breakout::Port);
        ops.push(Op::Access(Access {
            ports: narrow(tier.count, "server ports")?,
            speed,
            breakout,
        }));
    }

    for tier in [&upstream, &edge] {
        budgets(tier, &mut ops)?;
        // One tier written twice is one panel, not two.
        if upstream.word == edge.word {
            break;
        }
    }

    Ok(Request {
        switches,
        ops,
        shape,
        media: link.media.unwrap_or_default(),
    })
}

/// Which shape a pair of tiers makes, and whether they pair up at all.
fn pair(first: &Tier, second: &Tier) -> Result<Topology, String> {
    match (first.name, second.name) {
        (Name::Spines, Name::Leaves) => Ok(Topology::LeafSpine),
        (Name::Hub, Name::Spokes) => {
            if first.count != 1 {
                return Err(format!(
                    "a star has one hub, not {}: a fabric with more than one switch above the \
                     others is spines and leaves",
                    first.count
                ));
            }
            Ok(Topology::Star)
        }
        (Name::Servers, _) => Err("servers are the last tier, not the first".into()),
        (_, Name::Servers) => unreachable!("servers are read as the tail of the chain"),
        (Name::Switches, _) | (_, Name::Switches) => Err(
            "'switches' is the one tier of a mesh or a ring: two tiers are spines and leaves, \
             or a hub and spokes"
                .into(),
        ),
        (a, b) => Err(format!(
            "'{}' over '{}' is not a fabric: write spines over leaves, or a hub over spokes",
            word(a),
            word(b)
        )),
    }
}

fn word(name: Name) -> &'static str {
    match name {
        Name::Switches => "switches",
        Name::Spines => "spines",
        Name::Leaves => "leaves",
        Name::Hub => "hub",
        Name::Spokes => "spokes",
        Name::Servers => "servers",
    }
}

/// The port on a tier that carries a link of this speed, which is the
/// smallest one it has at or above that speed.
fn carrier(tier: &Tier, link: Option<Speed>) -> Result<Option<Speed>, String> {
    let Some(link) = link else {
        return Ok(None);
    };
    let mut speeds: Vec<Speed> = tier.panel.iter().filter_map(|(_, s)| *s).collect();
    if speeds.is_empty() {
        return Ok(None);
    }
    speeds.sort_unstable();
    match speeds.iter().find(|s| **s >= link) {
        Some(s) => Ok(Some(*s)),
        None => Err(format!(
            "no port on {} carries a {link} link: its ports are {}",
            tier.word,
            speeds
                .iter()
                .map(Speed::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// A panel entry with a count is a question about whether the plan fits it.
fn budgets(tier: &Tier, ops: &mut Vec<Op>) -> Result<(), String> {
    for (ports, speed) in &tier.panel {
        let Some(ports) = ports else {
            continue;
        };
        if *ports == 0 {
            return Err(format!(
                "a switch with 0 ports cannot be wired to anything: '{}'",
                tier.word
            ));
        }
        if *ports > MAX_PORTS {
            return Err(format!(
                "{ports} ports on one switch is past what this will plan \
                 ({MAX_PORTS} is the limit)"
            ));
        }
        ops.push(Op::Budget(Budget {
            ports: *ports,
            speed: *speed,
            role: tier.name.role(),
        }));
    }
    Ok(())
}

fn narrow(n: u64, what: &str) -> Result<u32, String> {
    u32::try_from(n).map_err(|_| format!("{n} {what} is past what this will plan"))
}

// ---------------------------------------------------------------------------
// The scanner. The grammar is small enough that a cursor reads better than a
// parser combinator, and it keeps the error messages pointing at the text.
// ---------------------------------------------------------------------------

struct Cursor<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Cursor<'a> {
        Cursor { text, at: 0 }
    }

    fn rest(&self) -> &'a str {
        &self.text[self.at..]
    }

    fn skip_space(&mut self) {
        let trimmed = self.rest().trim_start();
        self.at = self.text.len() - trimmed.len();
    }

    fn at_end(&mut self) -> bool {
        self.skip_space();
        self.rest().is_empty()
    }

    fn take_while(&mut self, mut ok: impl FnMut(char) -> bool) -> &'a str {
        let start = self.at;
        while let Some(c) = self.rest().chars().next() {
            if !ok(c) {
                break;
            }
            self.at += c.len_utf8();
        }
        &self.text[start..self.at]
    }

    fn eat(&mut self, c: char) -> bool {
        if self.rest().starts_with(c) {
            self.at += c.len_utf8();
            return true;
        }
        false
    }

    /// A shape word, when the far side of a link is one rather than a tier.
    fn shape_word(&mut self) -> Option<Topology> {
        self.skip_space();
        let save = self.at;
        let word = self.take_while(|c| c.is_ascii_alphabetic() || c == '-');
        match word {
            "mesh" => Some(Topology::Mesh),
            "ring" => Some(Topology::Ring),
            _ => {
                self.at = save;
                None
            }
        }
    }

    /// `4 spines[32x400G]`
    fn tier(&mut self) -> Result<Tier, String> {
        self.skip_space();
        let start = self.at;
        if self.at_end() {
            return Err("the fabric stops early: it needs a tier here, such as '8 leaves'".into());
        }
        let digits = self.take_while(|c| c.is_ascii_digit());
        if digits.is_empty() {
            let stray = self.take_while(|c| !c.is_whitespace() && c != '-');
            return Err(format!(
                "'{stray}' is not a tier: a tier is a count and a name, such as '8 leaves'"
            ));
        }
        let count: u64 = digits
            .parse()
            .map_err(|_| format!("'{digits}' is not a switch count"))?;
        self.skip_space();
        let name_word = self.take_while(|c| c.is_ascii_alphabetic());
        if name_word.is_empty() {
            return Err(format!(
                "'{digits}' needs to say what it counts: spines, leaves, switches, hub, spokes \
                 or servers"
            ));
        }
        let Some(name) = Name::parse(name_word) else {
            return Err(format!(
                "'{name_word}' is not a kind of tier: spines, leaves, switches, hub, spokes \
                 or servers"
            ));
        };
        if count == 0 {
            return Err(format!("a fabric needs {name_word} in it, not 0"));
        }
        let panel = self.panel()?;
        Ok(Tier {
            count,
            name,
            panel,
            word: self.text[start..self.at].trim().to_string(),
        })
    }

    /// `[32x400G]`, `[400G]`, `[32]`, or nothing at all.
    fn panel(&mut self) -> Result<Vec<Port>, String> {
        if !self.eat('[') {
            return Ok(Vec::new());
        }
        let body = self.take_while(|c| c != ']');
        if !self.eat(']') {
            return Err("a front panel needs its closing bracket: write it like [32x400G]".into());
        }
        let mut ports = Vec::new();
        for entry in body.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                return Err(
                    "a front panel has an empty entry: write it like [48x25G,4x100G]".into(),
                );
            }
            ports.push(port(entry)?);
        }
        Ok(ports)
    }

    /// `-100G-`, `-2x100G:dac-`, or `--` for a fabric with no speeds.
    fn link(&mut self) -> Result<Link, String> {
        self.skip_space();
        if !self.eat('-') {
            let stray = self.take_while(|c| !c.is_whitespace());
            return Err(format!(
                "'{stray}' is not a link: the tiers are joined by -100G-, or by -- when no \
                 speed is known yet"
            ));
        }
        let body = self.take_while(|c| c != '-');
        if !self.eat('-') {
            return Err("a link needs its closing dash: write it like -100G-".into());
        }
        let mut link = Link::default();
        let (rate, media) = match body.split_once(':') {
            Some((rate, media)) => (rate, Some(media)),
            None => (body, None),
        };
        if let Some(media) = media {
            link.media = Some(match media.trim() {
                "optic" => Media::Optic,
                "aoc" => Media::Aoc,
                "dac" => Media::Dac,
                other => {
                    return Err(format!(
                        "'{other}' is not what a link is made of: optic, aoc or dac"
                    ));
                }
            });
        }
        let rate = rate.trim();
        if rate.is_empty() {
            return Ok(link);
        }
        let (count, rate) = match rate.split_once(['x', 'X']) {
            Some((count, rate)) => {
                let k: u32 = count
                    .parse()
                    .map_err(|_| format!("'{count}' is not a number of links"))?;
                if k == 0 {
                    return Err("0 links between each pair is not a fabric".into());
                }
                (Some(k), rate)
            }
            None => (None, rate),
        };
        link.per_pair = count;
        // How many links there are is worth knowing before anyone has picked
        // a speed, so `-2x-` counts cables and says nothing about rates.
        if !rate.is_empty() {
            link.speed = Some(speed::parse(rate)?);
        }
        Ok(link)
    }

    fn finish(&mut self) -> Result<(), String> {
        if self.at_end() {
            return Ok(());
        }
        Err(format!(
            "'{}' is left over at the end of the fabric",
            self.rest().trim()
        ))
    }
}

/// One front-panel entry: a count, a speed, or both.
fn port(entry: &str) -> Result<Port, String> {
    if let Some((count, rate)) = entry.split_once(['x', 'X']) {
        let n: u32 = count
            .trim()
            .parse()
            .map_err(|_| format!("'{count}' is not a port count"))?;
        return Ok((Some(n), Some(speed::parse(rate.trim())?)));
    }
    if entry.bytes().all(|b| b.is_ascii_digit()) {
        let n: u32 = entry
            .parse()
            .map_err(|_| format!("'{entry}' is not a port count"))?;
        return Ok((Some(n), None));
    }
    Ok((None, Some(speed::parse(entry)?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(fabric: &str) -> Vec<Op> {
        parse(fabric).expect("parses").ops
    }

    fn speed(s: &str) -> Speed {
        speed::parse(s).unwrap()
    }

    fn err(fabric: &str) -> String {
        parse(fabric).expect_err("should not parse")
    }

    #[test]
    fn a_flat_fabric_is_a_tier_and_the_pattern_it_is_wired_in() {
        let r = parse("8 switches -100G- mesh").unwrap();
        assert_eq!((r.switches, r.shape), (8, Topology::Mesh));
        assert_eq!(r.ops, vec![Op::Speed(speed("100G"))]);
        assert_eq!(parse("24 switches -- ring").unwrap().shape, Topology::Ring);
    }

    /// Spines are written first because the chain runs from the core to the
    /// edge, which is also the order the servers arrive in.
    #[test]
    fn two_tiers_are_spines_over_leaves() {
        let r = parse("2 spines -100G- 16 leaves").unwrap();
        assert_eq!((r.switches, r.shape), (16, Topology::LeafSpine));
        assert!(r.ops.contains(&Op::Spines(2)));
    }

    #[test]
    fn a_hub_over_spokes_is_a_star() {
        let r = parse("1 hub -25G- 47 spokes").unwrap();
        assert_eq!((r.switches, r.shape), (48, Topology::Star));
        // A star counts its hub among its switches rather than apart.
        assert!(!r.ops.iter().any(|o| matches!(o, Op::Spines(_))));
        assert!(err("2 hubs -25G- 47 spokes").contains("a star has one hub"));
    }

    /// The port that carries a link is the smallest one at or above it, and
    /// the port being the faster of the two is what breaking out means.
    #[test]
    fn the_port_that_carries_a_link_is_the_smallest_that_can() {
        assert!(
            ops("8 switches[400G] -100G- mesh")
                .contains(&Op::Breakout(Breakout::Port(speed("400G"))))
        );
        // An exact match is a whole port, so nothing breaks out.
        assert!(
            !ops("8 switches[100G] -100G- mesh")
                .iter()
                .any(|o| matches!(o, Op::Breakout(_)))
        );
        // A leaf with ports at several speeds uses the one that fits best.
        assert!(
            !ops("2 spines -100G- 8 leaves[48x25G,2x200G,4x100G]")
                .iter()
                .any(|o| matches!(o, Op::Breakout(_)))
        );
        // Only the tier that breaks out is read for it.
        assert!(
            ops("2 spines[400G] -100G- 8 leaves")
                .contains(&Op::Breakout(Breakout::Port(speed("400G"))))
        );
        assert!(err("8 switches[100G] -400G- mesh").contains("no port on"));
    }

    #[test]
    fn a_panel_entry_may_give_either_half_or_both() {
        let b = |fabric| match ops(fabric).into_iter().find_map(|o| match o {
            Op::Budget(b) => Some(b),
            _ => None,
        }) {
            Some(b) => (b.ports, b.speed),
            None => (0, None),
        };
        assert_eq!(b("8 switches[32] -100G- mesh"), (32, None));
        assert_eq!(b("8 switches[400G] -100G- mesh"), (0, None));
        assert_eq!(
            b("8 switches[32x400G] -100G- mesh"),
            (32, Some(speed("400G")))
        );
    }

    /// A panel belongs to the tier it is written on, which is what makes a
    /// question about one kind of switch rather than whatever matches.
    #[test]
    fn a_panel_carries_the_role_of_its_tier() {
        let roles: Vec<_> = ops("2 spines[32] -100G- 8 leaves[48]")
            .into_iter()
            .filter_map(|o| match o {
                Op::Budget(b) => b.role,
                _ => None,
            })
            .collect();
        assert_eq!(roles, vec![Role::Spine, Role::Leaf]);
    }

    #[test]
    fn a_link_carries_its_media_and_its_parallel_links() {
        assert_eq!(
            parse("8 switches -100G:dac- mesh").unwrap().media,
            Media::Dac
        );
        assert_eq!(
            parse("8 switches -100G:aoc- mesh").unwrap().media,
            Media::Aoc
        );
        assert_eq!(parse("8 switches -100G- mesh").unwrap().media, Media::Optic);
        assert!(ops("8 switches -2x100G- mesh").contains(&Op::PerPair(2)));
        // How many links there are is worth knowing before a speed is picked.
        assert!(ops("8 switches -3x- mesh").contains(&Op::PerPair(3)));
        assert!(err("8 switches -100G:copper- mesh").contains("optic, aoc or dac"));
    }

    #[test]
    fn servers_are_the_tier_the_chain_ends_with() {
        let a = ops("2 spines -100G- 8 leaves -25G- 48 servers")
            .into_iter()
            .find_map(|o| match o {
                Op::Access(a) => Some(a),
                _ => None,
            })
            .expect("servers");
        assert_eq!((a.ports, a.speed), (48, speed("25G")));
        assert_eq!(a.breakout, None);
        // Servers arrive broken out when the leaf has no port their size.
        let a = ops("2 spines -100G- 8 leaves[100G] -25G- 48 servers")
            .into_iter()
            .find_map(|o| match o {
                Op::Access(a) => Some(a),
                _ => None,
            })
            .expect("servers");
        assert_eq!(a.breakout, Some(Breakout::Port(speed("100G"))));
        // A mesh has servers too: its every switch is the fabric's edge.
        assert!(
            ops("8 switches -100G- mesh -25G- 48 servers")
                .iter()
                .any(|o| matches!(o, Op::Access(_)))
        );
    }

    /// Three tiers of switches is a super-spine layer, which this does not
    /// plan. The message says what it is rather than counting tiers.
    #[test]
    fn a_third_tier_of_switches_is_named_for_what_it_is() {
        let e = err("2 spines -400G- 8 spines -100G- 16 leaves");
        assert!(e.contains("super-spine layer"), "{e}");
        assert!(e.contains("two fabrics"), "{e}");
    }

    #[test]
    fn what_cannot_be_a_fabric_is_turned_down() {
        for (fabric, says) in [
            ("", "needs a tier"),
            ("8", "what it counts"),
            ("8 racks -100G- mesh", "not a kind of tier"),
            ("0 switches -100G- mesh", "not 0"),
            ("8 leaves -100G- mesh", "one tier of switches"),
            ("2 spines -100G- 8 switches", "one tier of a mesh"),
            ("8 leaves -100G- 2 spines", "spines over leaves"),
            ("8 switches 100G mesh", "is not a link"),
            ("8 switches -100G mesh", "closing dash"),
            ("8 switches[32 -100G- mesh", "closing bracket"),
            ("8 switches[] -100G- mesh", "empty entry"),
            ("8 switches -100G- mesh extra", "is not a link"),
            ("8 switches -100G- mesh -25G- 48 servers junk", "left over"),
            ("2 spines -100G- 8 leaves -- 48 servers", "needs a speed"),
        ] {
            let e = err(fabric);
            assert!(e.contains(says), "{fabric:?}: {e}");
        }
    }

    /// Whitespace is for reading by, so the notation does not depend on it.
    #[test]
    fn spacing_is_not_load_bearing() {
        let tight = parse("4spines[32x400G]-100G-8leaves[48x25G]-25G-48servers").unwrap();
        let loose = parse("4 spines[32x400G] -100G- 8 leaves[48x25G] -25G- 48 servers").unwrap();
        assert_eq!(tight, loose);
    }
}
