//! fabrictool - how many cables, transceivers and ports a set of switches
//! needs to be wired to each other.
//!
//! The question this answers is the one that comes up when a rack of switches
//! is being specified: eight of these, all talking to each other at 100G, out
//! of 400G ports - how many optics is that, how many breakout harnesses, and
//! do the ports even fit? The counting is not hard, but it is fiddly enough
//! to get wrong on a whiteboard, and getting it wrong means a purchase order
//! that is short by eight transceivers.
//!
//! | Module | Holds |
//! | --- | --- |
//! | `speed.rs` | Port and link rates, and how they are written |
//! | `ops.rs` | The operator grammar |
//! | `plan.rs` | Topology, ports, cables and bandwidth |
//! | `schedule.rs` | Which port on which switch reaches which |
//! | `render.rs` | Text, `--quiet` and `--json` output |
//! | `dot.rs` | A Graphviz drawing of the fabric |

pub mod dot;
pub mod ops;
pub mod plan;
pub mod render;
pub mod schedule;
pub mod speed;

use std::fmt;

/// How the switches are wired to each other.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Topology {
    /// Every switch to every other. The default, because it is the shape
    /// whose cable count is most often miscalculated.
    #[default]
    #[value(alias = "full", alias = "full-mesh")]
    Mesh,
    /// Each switch to two neighbours, closing back on itself.
    #[value(alias = "loop")]
    Ring,
    /// One switch in the middle, everything else connected to it.
    #[value(alias = "hub", alias = "hub-and-spoke")]
    Star,
    /// Every leaf to every spine, and nothing to anything else. The shape a
    /// rack of switches with servers under them actually has.
    #[value(alias = "clos", alias = "spine-leaf")]
    LeafSpine,
}

impl Topology {
    pub fn name(&self) -> &'static str {
        match self {
            Topology::Mesh => "full mesh",
            Topology::Ring => "ring",
            Topology::Star => "star",
            Topology::LeafSpine => "leaf-spine",
        }
    }

    /// The parenthesised half of the topology line: what the shape means,
    /// rather than what it is called.
    pub fn describe(&self) -> &'static str {
        match self {
            Topology::Mesh => "every switch to every other",
            Topology::Ring => "each switch to two neighbours",
            Topology::Star => "one hub, everything else connected to it",
            Topology::LeafSpine => "every leaf to every spine",
        }
    }
}

impl fmt::Display for Topology {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl Topology {
    /// Whether the switches all have the same job. A mesh and a ring are one
    /// population; a star and a leaf-spine are two, and almost every number
    /// in the report has to be given per population rather than per switch.
    pub fn is_uniform(&self) -> bool {
        matches!(self, Topology::Mesh | Topology::Ring)
    }
}

/// What a switch in a given part of the fabric has to hold. A mesh and a ring
/// have one of these; a star has two, because its hub and its spokes are not
/// the same shape.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Role {
    Every,
    Hub,
    Spoke,
    Spine,
    Leaf,
}

impl Role {
    pub fn label(&self) -> &'static str {
        match self {
            Role::Every => "each switch",
            Role::Hub => "the hub",
            Role::Spoke => "each spoke",
            Role::Spine => "each spine",
            Role::Leaf => "each leaf",
        }
    }

    /// The role as a bare noun, for a label that supplies its own article:
    /// "Per leaf" rather than "Per each leaf".
    pub fn singular(&self) -> &'static str {
        match self {
            Role::Every => "switch",
            Role::Hub => "hub",
            Role::Spoke => "spoke",
            Role::Spine => "spine",
            Role::Leaf => "leaf",
        }
    }

    pub fn key(&self) -> &'static str {
        self.singular()
    }

    /// A role as somebody writes it when naming one: singular or plural, and
    /// `switch` for the shapes where every switch is the same.
    pub fn parse(s: &str) -> Option<Role> {
        match s.to_ascii_lowercase().as_str() {
            "switch" | "switches" | "every" | "all" => Some(Role::Every),
            "hub" | "hubs" => Some(Role::Hub),
            "spoke" | "spokes" => Some(Role::Spoke),
            "spine" | "spines" => Some(Role::Spine),
            "leaf" | "leaves" | "leafs" => Some(Role::Leaf),
            _ => None,
        }
    }

    /// Every name this accepts, for an error that has to list them.
    pub fn names() -> &'static str {
        "switch, hub, spoke, spine or leaf"
    }
}

/// What the link is physically made of.
///
/// This is not decoration: it decides whether there are transceivers to count
/// at all. A DAC or an AOC arrives as one assembly with its ends already
/// attached, so the bill of materials is the cable and nothing else.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Media {
    /// Transceivers in the ports, fibre between them. The default, because it
    /// is the only one of the three that reaches across a room.
    #[default]
    Optic,
    /// Active optical cable: fibre with the ends moulded on.
    Aoc,
    /// Direct attach copper: a passive twinax lead with the ends moulded on.
    Dac,
}

impl Media {
    pub fn name(&self) -> &'static str {
        match self {
            Media::Optic => "optic",
            Media::Aoc => "AOC",
            Media::Dac => "DAC",
        }
    }

    /// Whether the ends are separate parts that have to be bought.
    pub fn has_transceivers(&self) -> bool {
        matches!(self, Media::Optic)
    }
}

impl fmt::Display for Media {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
