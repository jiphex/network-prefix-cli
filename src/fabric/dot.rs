//! This module draws the fabric as a Graphviz graph, for looking at rather
//! than reading.
//!
//! The report says what a fabric costs and the schedule says where each cable
//! goes; neither shows the shape. `dot -Tpng` does, and a graph is the one
//! form in which a mesh of sixteen and a leaf-spine of sixteen are obviously
//! not the same thing.
//!
//! It draws one edge per pair of switches by default, labelled with what runs
//! between them, because a diagram of a fabric is about the shape rather than
//! about individual cables. With `--schedule` it draws a cable at a time
//! instead, labelled with the ports each end occupies, which is the same list
//! the schedule prints and easier to check against a rack.

use super::Topology;
use super::plan::{Report, Role, Side};
use super::schedule::{self, Schedule};
use std::io::{self, Write};

pub fn write(w: &mut impl Write, r: &Report) -> io::Result<()> {
    let p = &r.plan;
    let title = match p.speed {
        Some(s) => format!("{} - {} at {s}", subject(r), p.topology),
        None => format!("{} - {}", subject(r), p.topology),
    };
    writeln!(w, "// fabrictool: {title}")?;
    writeln!(w, "graph fabric {{")?;
    writeln!(
        w,
        "  graph [rankdir=TB, labelloc=t, fontname=\"Helvetica\", label=\"{}\"];",
        escape(&title)
    )?;
    writeln!(
        w,
        "  node [shape=box, style=rounded, fontname=\"Helvetica\", fontsize=11];"
    )?;
    writeln!(w, "  edge [fontname=\"Helvetica\", fontsize=9];")?;

    // A shape with two kinds of switch draws them as two ranks, which is how
    // anybody sketches it on a whiteboard.
    for side in &p.sides {
        let members: Vec<u64> = members(p.switches, p.spines, side);
        if p.topology.is_uniform() {
            for i in members {
                writeln!(w, "  {};", node(r, i, side))?;
            }
            continue;
        }
        writeln!(w, "\n  subgraph cluster_{} {{", side.role.key())?;
        writeln!(
            w,
            "    label=\"{}\"; style=dashed; color=\"#999999\"; fontsize=10;",
            escape(&plural(side.switches, side.role.singular()))
        )?;
        for i in members {
            writeln!(w, "    {};", node(r, i, side))?;
        }
        writeln!(w, "  }}")?;
    }
    writeln!(w)?;

    if r.schedule {
        // A cable at a time, each labelled with the ports it occupies.
        for patch in Schedule::new(p) {
            writeln!(
                w,
                "  {} -- {} [label=\"{}:{} - {}:{}\"];",
                patch.a.name(),
                patch.b.name(),
                patch.a.prefix_number(),
                port(patch.a.port, patch.a.lane),
                patch.b.prefix_number(),
                port(patch.b.port, patch.b.lane),
            )?;
        }
        writeln!(w, "}}")?;
        return Ok(());
    }

    let label = match (p.speed, p.per_pair) {
        (Some(s), 1) => s.to_string(),
        (Some(s), k) => format!("{k} x {s}"),
        (None, k) => plural(k, "link"),
    };
    for (a, b) in schedule::pairs(p) {
        writeln!(
            w,
            "  {} -- {} [label=\"{}\"];",
            schedule::name(p, a),
            schedule::name(p, b),
            escape(&label)
        )?;
    }
    writeln!(w, "}}")
}

/// The switches of one role, by their index in the plan's own order.
fn members(switches: u64, spines: u64, side: &Side) -> Vec<u64> {
    match side.role {
        Role::Hub => vec![0],
        Role::Spoke => (1..switches).collect(),
        Role::Spine => (0..spines).collect(),
        Role::Leaf => (spines..switches).collect(),
        Role::Every => (0..switches).collect(),
    }
}

/// A node, labelled with what that switch gives up: its fabric ports, and the
/// servers under it when there are any.
fn node(r: &Report, index: u64, side: &Side) -> String {
    let p = &r.plan;
    let name = schedule::name(p, index);
    // Each line is escaped on its own and joined afterwards, because the
    // separator DOT wants between them is itself a backslash.
    let mut lines = vec![name.clone()];
    lines.push(match side.port_speed {
        Some(s) => format!("{} x {s}", side.ports),
        None => plural(side.ports, "port"),
    });
    if side.lanes > 1 {
        let last = lines.len() - 1;
        lines[last].push_str(&format!(" ({} lanes each)", side.lanes));
    }
    if let Some(a) = side.access {
        lines.push(format!("{} x {} servers", a.servers, a.speed));
    }
    let label = lines
        .iter()
        .map(|l| escape(l))
        .collect::<Vec<_>>()
        .join("\\n");
    let shape = match (p.topology, side.role) {
        // The one switch everything else depends on is drawn as the one
        // switch everything else depends on.
        (Topology::Star, Role::Hub) => ", shape=box3d",
        _ => "",
    };
    format!("{name} [label=\"{label}\"{shape}]")
}

fn port(port: u64, lane: Option<u64>) -> String {
    match lane {
        Some(lane) => format!("{port}/{lane}"),
        None => port.to_string(),
    }
}

fn subject(r: &Report) -> String {
    let p = &r.plan;
    match p.spines {
        0 => plural(p.switches, "switch"),
        spines => format!(
            "{} + {}",
            plural(p.switches - spines, "leaf"),
            plural(spines, "spine")
        ),
    }
}

fn plural(n: u64, what: &str) -> String {
    let many = match what {
        "switch" => "switches".to_string(),
        "leaf" => "leaves".to_string(),
        w => format!("{w}s"),
    };
    format!("{n} {}", if n == 1 { what.to_string() } else { many })
}

/// DOT strings are quoted, so a quote or a backslash in one has to be spelled
/// out. Nothing here generates either today, which is exactly when it is
/// cheap to get right.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fabric::ops;
    use crate::fabric::plan::{self, Options};

    fn dot(switches: u64, args: &[&str], opts: Options) -> String {
        let ops: Vec<_> = args.iter().map(|a| ops::parse(a).unwrap()).collect();
        let r = plan::build(switches, &ops, opts).expect("plans");
        let mut out = Vec::new();
        write(&mut out, &r).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn plain(switches: u64, args: &[&str]) -> String {
        dot(switches, args, Options::default())
    }

    #[test]
    fn a_mesh_is_nodes_and_edges() {
        let s = plain(3, &["@100G"]);
        assert!(s.starts_with("// fabrictool: 3 switches - full mesh at 100G\n"));
        assert!(s.contains("graph fabric {"), "{s}");
        assert!(s.contains("sw1 [label=\"sw1\\n2 x 100G\"]"), "{s}");
        assert!(s.contains("  sw1 -- sw2 [label=\"100G\"];"), "{s}");
        assert!(s.contains("  sw2 -- sw3 [label=\"100G\"];"), "{s}");
        assert!(s.trim_end().ends_with('}'), "{s}");
        // There is one edge per pair, not one per switch.
        assert_eq!(s.matches(" -- ").count(), 3);
        // Nothing to cluster when every switch is the same.
        assert!(!s.contains("subgraph"), "{s}");
    }

    #[test]
    fn a_leaf_spine_draws_its_two_ranks() {
        let s = dot(4, &["+2", "@100G", "%400G", "-48@25G"], Options::default());
        assert!(s.contains("subgraph cluster_spine {"), "{s}");
        assert!(s.contains("label=\"2 spines\""), "{s}");
        assert!(s.contains("subgraph cluster_leaf {"), "{s}");
        assert!(s.contains("label=\"4 leaves\""), "{s}");
        assert!(
            s.contains("spine1 [label=\"spine1\\n1 x 400G (4 lanes each)\"]"),
            "{s}"
        );
        assert!(
            s.contains("leaf1 [label=\"leaf1\\n2 x 100G\\n48 x 25G servers\"]"),
            "{s}"
        );
        // Every leaf reaches every spine, which is eight edges, and no leaf
        // reaches another.
        assert_eq!(s.matches(" -- ").count(), 8);
        assert!(!s.contains("leaf1 -- leaf2"), "{s}");
    }

    #[test]
    fn parallel_links_are_one_labelled_edge() {
        let s = plain(3, &["x3", "@400G"]);
        assert_eq!(s.matches(" -- ").count(), 3);
        assert!(s.contains("[label=\"3 x 400G\"]"), "{s}");
        // And with no speed at all, the count still says something.
        let s = plain(3, &["x2"]);
        assert!(s.contains("[label=\"2 links\"]"), "{s}");
    }

    #[test]
    fn the_schedule_draws_a_cable_at_a_time() {
        let s = dot(
            4,
            &["+2", "@100G", "%400G"],
            Options {
                schedule: true,
                ..Options::default()
            },
        );
        assert!(
            s.contains("spine1 -- leaf1 [label=\"spine1:1/1 - leaf1:1\"];"),
            "{s}"
        );
        assert!(
            s.contains("spine2 -- leaf4 [label=\"spine2:1/4 - leaf4:2\"];"),
            "{s}"
        );
        assert_eq!(s.matches(" -- ").count(), 8);
    }

    #[test]
    fn a_star_marks_the_switch_everything_depends_on() {
        let s = dot(
            4,
            &["@100G"],
            Options {
                shape: Some(Topology::Star),
                ..Options::default()
            },
        );
        assert!(s.contains("shape=box3d"), "{s}");
        assert_eq!(s.matches(" -- ").count(), 3);
    }

    #[test]
    fn every_switch_appears_exactly_once() {
        for (switches, args, opts) in [
            (6u64, vec!["@100G"], Options::default()),
            (6, vec!["@100G", "x2"], Options::default()),
            (
                6,
                vec!["@100G"],
                Options {
                    shape: Some(Topology::Ring),
                    ..Options::default()
                },
            ),
            (6, vec!["+3", "@100G", "-24@10G"], Options::default()),
        ] {
            let s = dot(switches, &args, opts);
            // Every `[label="` is a node or an edge; the graph's own label is
            // not one of them.
            let declared = s.matches("[label=\"").count() - s.matches(" -- ").count();
            let plan_switches = if args.iter().any(|a| a.starts_with('+')) {
                switches + 3
            } else {
                switches
            };
            assert_eq!(declared, plan_switches as usize, "{args:?}\n{s}");
        }
    }
}
