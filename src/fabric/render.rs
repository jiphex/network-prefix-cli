//! This module formats the report three ways: for a person to read, for a
//! machine to parse, and as a bare list for piping into other tools.
//!
//! The layout follows the prefix side of this repository on purpose - a
//! two-space indent, a padded label, a section heading with a blank line
//! before it - because the two are meant to sit in the same terminal without
//! looking like different programs. The same rules apply as well: labels are
//! padded before they are styled, because an escape sequence has no printed
//! width but the formatter counts it anyway, and `--quiet` and `--json` are
//! never coloured, whatever `--color` says.

use super::plan::{Arrangement, Budget, Item, Plan, Ratio, Report, Side};
use super::schedule::Schedule;
use super::speed::Speed;
use super::{Media, Topology};
use crate::json::{self, J};
use crate::num;
use crate::style::Style;
use std::io::{self, Write};

pub struct Opts {
    /// How many links to list in the patch schedule, when the reader asked
    /// for fewer than all of them.
    pub limit: Option<usize>,
    /// Colour for the human-readable report. Machine output ignores it.
    pub style: Style,
}

impl Opts {
    fn take(&self) -> usize {
        self.limit.unwrap_or(usize::MAX)
    }
}

const LABEL: usize = 15;

pub fn text(w: &mut impl Write, r: &Report, o: &Opts) -> io::Result<()> {
    let p = &r.plan;
    writeln!(
        w,
        "{}  {}  {}",
        o.style.title(&subject(p)),
        o.style.dim("-"),
        headline(p),
    )?;
    writeln!(w)?;

    field(
        w,
        o,
        "Switches",
        &match p.spines {
            0 => num::group(&p.switches.to_string()),
            spines => format!(
                "{}  {}",
                num::group(&p.switches.to_string()),
                o.style.dim(&format!(
                    "({}, {})",
                    plural(p.switches - spines, "leaf"),
                    plural(spines, "spine")
                ))
            ),
        },
    )?;
    field(
        w,
        o,
        "Topology",
        &format!(
            "{}  {}",
            p.topology,
            o.style.dim(&format!("({})", p.topology.describe()))
        ),
    )?;
    field(
        w,
        o,
        "Links",
        &format!(
            "{}  {}",
            num::group(&p.links.to_string()),
            o.style.dim(&format!(
                "({} {})",
                plural(p.per_pair, "link"),
                between(p.topology)
            ))
        ),
    )?;
    if let Some(s) = p.speed {
        field(w, o, "Link speed", &s.to_string())?;
        // With one link to a pair this row would only repeat the link speed,
        // so it appears when there is more than one to add up.
        if p.per_pair > 1 {
            field(
                w,
                o,
                &format!("Per {}", pair(p.topology)),
                &format!(
                    "{}  {}",
                    s.total(p.per_pair),
                    o.style.dim(&format!("({} x {s})", p.per_pair))
                ),
            )?;
        }
    }

    let multi = p.sides.len() > 1;
    if let Some(sp) = p.speed {
        rows(
            w,
            o,
            "Per switch",
            p.sides
                .iter()
                .map(|s| {
                    let links = plural(s.degree, "link");
                    let capacity = sp.total(s.degree);
                    match multi {
                        false => format!("{links}, {capacity}"),
                        true => format!("{} {links}, {capacity}", s.role.label_for(s.switches)),
                    }
                })
                .collect(),
        )?;
    }
    rows(
        w,
        o,
        "Ports",
        p.sides.iter().map(|s| ports(s, multi, o)).collect(),
    )?;
    let servers: Vec<String> = p
        .sides
        .iter()
        .filter_map(|s| {
            let a = s.access?;
            Some(port_line(
                a.ports,
                Some(a.port_speed),
                &whose(s, multi),
                a.lanes,
                a.servers,
                a.spare_lanes,
                o,
            ))
        })
        .collect();
    if !servers.is_empty() {
        rows(w, o, "Server ports", servers)?;
    }

    field(
        w,
        o,
        "Bisection",
        &match p.speed {
            Some(s) => format!(
                "{}  {}",
                s.total(p.bisection),
                o.style.dim(&format!(
                    "({} across the middle)",
                    plural(p.bisection, "link")
                ))
            ),
            None => format!("{} across the middle", plural(p.bisection, "link")),
        },
    )?;
    if let Some(s) = p.speed {
        field(
            w,
            o,
            "Fabric total",
            &format!(
                "{}  {}",
                s.total(p.links),
                o.style.dim(&format!(
                    "({} x {s}, one direction)",
                    num::group(&p.links.to_string())
                ))
            ),
        )?;
    }
    field(
        w,
        o,
        "Hops",
        &format!("{}  {}", p.hops, o.style.dim(&format!("({})", path(p)))),
    )?;
    field(w, o, "Resilience", &resilience(p))?;
    // The reason anyone buys the second spine, said in the terms they buy it
    // in: what a leaf still has when one of them is gone.
    if let Some(loss) = p.spine_loss() {
        field(
            w,
            o,
            "Spine loss",
            &match p.speed {
                Some(sp) => format!(
                    "each leaf keeps {} of {}  {}",
                    plural(loss.left, "uplink"),
                    loss.uplinks,
                    o.style.dim(&format!(
                        "({} of {})",
                        sp.total(loss.left),
                        sp.total(loss.uplinks)
                    ))
                ),
                None => format!(
                    "each leaf keeps {} of {}",
                    plural(loss.left, "uplink"),
                    loss.uplinks
                ),
            },
        )?;
    }
    for c in &p.cautions {
        field(w, o, "Caution", &o.style.warn(c))?;
    }

    cabling(w, p, o)?;
    oversubscription(w, p, o)?;
    for b in &r.budgets {
        budget(w, p, b, o)?;
    }
    if r.schedule {
        schedule(w, p, o)?;
    }
    Ok(())
}

/// The left-hand half of the opening line, naming what was asked about. A
/// leaf-spine is two populations and says so, because its switch count is not
/// the number anybody typed.
fn subject(p: &Plan) -> String {
    match p.spines {
        0 => plural(p.switches, "switch"),
        spines => format!(
            "{} + {}",
            plural(p.switches - spines, "leaf"),
            plural(spines, "spine")
        ),
    }
}

/// The right-hand half of the opening line, giving the shape and the speed.
fn headline(p: &Plan) -> String {
    match p.speed {
        Some(s) => format!("{} at {s}", p.topology),
        None => p.topology.to_string(),
    }
}

/// What a link joins, which is not the same sentence in every shape.
fn pair(topology: Topology) -> &'static str {
    match topology {
        Topology::Mesh => "pair",
        Topology::Ring => "neighbour",
        Topology::Star => "spoke",
        Topology::LeafSpine => "uplink",
    }
}

fn between(topology: Topology) -> &'static str {
    match topology {
        Topology::Mesh => "between each pair",
        Topology::Ring => "to each neighbour",
        Topology::Star => "from the hub to each spoke",
        Topology::LeafSpine => "from each leaf to each spine",
    }
}

/// The longest path through the fabric, said as the switches it crosses
/// rather than as a number on its own.
fn path(p: &Plan) -> &'static str {
    match p.topology {
        Topology::Mesh => "every switch reaches every other directly",
        Topology::Ring if p.switches == 2 => "one switch to the other",
        Topology::Ring => "the far side of the ring, one neighbour at a time",
        Topology::Star if p.switches == 2 => "the hub to its spoke",
        Topology::Star => "spoke to hub to spoke",
        Topology::LeafSpine => "leaf to spine to leaf",
    }
}

fn resilience(p: &Plan) -> String {
    match p.resilience {
        1 => "any single link failure splits the fabric".into(),
        n => format!(
            "{} may fail before the fabric splits",
            plural(n - 1, "link")
        ),
    }
}

/// What one switch of a given kind gives up to the fabric, and what is left
/// over in it.
fn ports(side: &Side, multi: bool, o: &Opts) -> String {
    port_line(
        side.ports,
        side.port_speed,
        &whose(side, multi),
        side.lanes,
        side.degree,
        side.spare_lanes,
        o,
    )
}

/// Whose ports a line is about. With one kind of switch there is nothing to
/// distinguish, so it stays the plain "each switch" it always was.
fn whose(side: &Side, multi: bool) -> String {
    match multi {
        false => " on each switch".to_string(),
        true => format!(" on {}", side.role.label_for(side.switches)),
    }
}

/// A count of ports at a speed, with the lanes inside them when they are
/// broken out. Shared by the fabric-facing and server-facing lines, because
/// they are the same arithmetic pointed in opposite directions.
fn port_line(
    ports: u64,
    speed: Option<Speed>,
    who: &str,
    lanes: u64,
    used: u64,
    spare: u64,
    o: &Opts,
) -> String {
    let each = match speed {
        Some(s) => format!("{} x {s}", num::group(&ports.to_string())),
        None => plural(ports, "port"),
    };
    let inside = if lanes > 1 {
        o.style.dim(&format!(
            "  ({lanes} lanes each: {} used, {} spare)",
            num::group(&used.to_string()),
            num::group(&spare.to_string()),
        ))
    } else {
        String::new()
    };
    format!("{each}{who}{inside}")
}

/// What is attached against what leaves - printed only when servers were
/// mentioned, because without them there is no ratio to have.
fn oversubscription(w: &mut impl Write, p: &Plan, o: &Opts) -> io::Result<()> {
    let with_access: Vec<&Side> = p.sides.iter().filter(|s| s.access.is_some()).collect();
    let Some(&side) = with_access.first() else {
        return Ok(());
    };
    heading(w, o, "Oversubscription")?;
    let access = side.access.expect("filtered for it");
    for s in &with_access {
        let a = s.access.expect("filtered for it");
        let label = format!("Per {}", s.role.singular());
        match s.ratio(p.speed) {
            Some(ratio) => field(
                w,
                o,
                &label,
                &format!(
                    "{}  {}",
                    verdict(&ratio),
                    o.style.dim(&format!(
                        "({} attached against {} of fabric{})",
                        a.speed.total(a.servers),
                        p.speed
                            .map_or(String::new(), |sp| sp.total(s.degree).to_string()),
                        if ratio.blocking() {
                            ""
                        } else {
                            " - non-blocking"
                        }
                    ))
                ),
            )?,
            // Without a link speed there is a count but no ratio, and a ratio
            // is what the section is for.
            None => field(
                w,
                o,
                &label,
                &format!(
                    "{} attached, fabric unknown  {}",
                    a.speed.total(a.servers),
                    o.style.dim("(add @100G for a ratio)")
                ),
            )?,
        }
    }
    // A ratio that only holds while every spine is up is half a ratio.
    if let (Some(loss), Some(leaves)) = (p.spine_loss(), p.leaves())
        && let Some(degraded) = leaves.ratio_over(p.speed, loss.left)
    {
        field(
            w,
            o,
            "One spine down",
            &format!(
                "{}  {}",
                verdict(&degraded),
                o.style.dim(&format!(
                    "({} of fabric left on each leaf)",
                    p.speed
                        .map_or(String::new(), |sp| sp.total(loss.left).to_string())
                ))
            ),
        )?;
    }
    let total: u64 = with_access
        .iter()
        .map(|s| s.switches * s.access.expect("filtered for it").servers)
        .sum();
    field(
        w,
        o,
        "Servers",
        &format!(
            "{} x {}  {}",
            num::group(&total.to_string()),
            access.speed,
            o.style.dim(&format!(
                "({} attached in total)",
                access.speed.total(total)
            ))
        ),
    )?;
    Ok(())
}

/// The ratio itself, as how many times more is attached than can leave. It is
/// written the way it is said, with the bigger side first and always against
/// one.
fn verdict(ratio: &Ratio) -> String {
    if ratio.blocking() {
        return format!("{}:1", scaled(ratio.down, ratio.up));
    }
    format!("1:{}", scaled(ratio.up, ratio.down))
}

/// `a` over `b` to two decimal places, without going through a float and
/// without trailing zeroes: 3, or 2.4, rather than 3.00 and 2.40.
fn scaled(a: u64, b: u64) -> String {
    if b == 0 {
        return "0".into();
    }
    let hundredths = a.saturating_mul(100) / b;
    let (whole, frac) = (hundredths / 100, hundredths % 100);
    if frac == 0 {
        return num::group(&whole.to_string());
    }
    let frac = format!("{frac:02}");
    format!(
        "{}.{}",
        num::group(&whole.to_string()),
        frac.trim_end_matches('0')
    )
}

/// How many switches the trunk-port count is spread over, when that is more
/// than one. The line above it in the report counts ports on one switch, so
/// this one has to say that it is counting the whole fabric.
fn trunk_whose(p: &Plan) -> String {
    let (count, whose) = match (p.arrangement, p.topology) {
        (Arrangement::SplitUpstream, Topology::LeafSpine) => (p.spines, "spines"),
        (Arrangement::SplitUpstream, _) => (1, "switches"),
        _ => (p.switches, "switches"),
    };
    if count > 1 {
        format!("across {count} {whose}: ")
    } else {
        String::new()
    }
}

fn cabling(w: &mut impl Write, p: &Plan, o: &Opts) -> io::Result<()> {
    heading(
        w,
        o,
        &match p.speed {
            Some(s) => format!("Cabling {} at {s}", subject(p)),
            None => format!("Cabling {}", subject(p)),
        },
    )?;
    field(w, o, "Arrangement", &arrangement(p))?;
    if p.lanes > 1 {
        let lanes = p.trunk_ports * p.lanes;
        field(
            w,
            o,
            match (p.arrangement, p.topology) {
                (Arrangement::SplitUpstream, Topology::LeafSpine) => "Spine ports",
                (Arrangement::SplitUpstream, _) => "Hub ports",
                _ => "Trunk ports",
            },
            &format!(
                "{}  {}",
                num::group(&p.trunk_ports.to_string()),
                o.style.dim(&format!(
                    "({}{} lanes, {} used, {} spare)",
                    trunk_whose(p),
                    num::group(&lanes.to_string()),
                    num::group(&(lanes - p.spare_lanes).to_string()),
                    num::group(&p.spare_lanes.to_string()),
                ))
            ),
        )?;
    }
    writeln!(w)?;

    let qty = p
        .materials
        .iter()
        .map(|i| num::group(&i.quantity.to_string()).len())
        .max()
        .unwrap_or(3)
        .max(3);
    let item = p
        .materials
        .iter()
        .map(|i| i.label.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let where_col = p
        .materials
        .iter()
        .map(|i| i.note.len())
        .max()
        .unwrap_or(5)
        .max(5);
    writeln!(
        w,
        "  {}",
        o.style
            .dim(&format!("{:>qty$}  {:<item$}  Where", "Qty", "Item"))
    )?;
    // A rule under the header, because the columns are wide enough by now
    // that a reader has to trace across them.
    writeln!(
        w,
        "  {}",
        o.style.dim(&format!(
            "{}  {}  {}",
            "-".repeat(qty),
            "-".repeat(item),
            "-".repeat(where_col)
        ))
    )?;
    for i in &p.materials {
        // Padded before styling, so the columns line up whether or not the
        // escapes are there.
        let count = format!("{:>qty$}", num::group(&i.quantity.to_string()));
        let label = format!("{:<item$}", i.label);
        writeln!(
            w,
            "  {}  {label}  {}",
            o.style.good(&count),
            o.style.dim(&i.note)
        )?;
    }
    Ok(())
}

/// The sentence that says where the splitters are, which is the part of the
/// plan a reader most needs to agree with before trusting the counts.
fn arrangement(p: &Plan) -> String {
    let link = p.speed.map_or(String::new(), |s| format!("{s} "));
    let port = p.port_speed.map_or("the port".into(), |s| s.to_string());
    match (p.arrangement, p.media) {
        (Arrangement::Straight, Media::Optic) => {
            format!("one {link}cable per link, a transceiver in each end")
        }
        (Arrangement::Straight, m) => format!("one {link}{m} per link, ends attached"),
        (Arrangement::SplitBothEnds, _) => format!(
            "{port} ports split {} ways at both ends, lanes joined in a patch field",
            p.lanes
        ),
        (Arrangement::SplitUpstream, _) => {
            let (upstream, downstream) = match p.topology {
                Topology::LeafSpine => ("each spine's", "leaf"),
                _ => ("the hub's", "spoke"),
            };
            format!(
                "{upstream} {port} ports split {} ways, a whole {link}port at each {downstream}",
                p.lanes
            )
        }
    }
}

fn budget(w: &mut impl Write, p: &Plan, b: &Budget, o: &Opts) -> io::Result<()> {
    heading(
        w,
        o,
        // Naming the kind of switch in the heading is what makes a stack of
        // these readable: "on a 4-port leaf" rather than four "on a ...-port
        // switch" in a row.
        &{
            let switch = b.role.map_or("switch", |r| r.singular());
            match b.speed {
                Some(at) => format!("{at} ports on a {}-port {switch}", b.ports),
                None => format!("Ports on a {}-port {switch}", b.ports),
            }
        },
    )?;
    let verdict = if b.fits() {
        o.style.good("yes")
    } else {
        o.style.bad("no")
    };
    let why = match b.sides.iter().find(|s| s.short > 0) {
        // A question about a kind of port nothing uses is answered rather
        // than left to be inferred from an empty list.
        None if b.sides.is_empty() => match (b.speed, b.role) {
            (Some(at), Some(role)) => format!("{} uses no port at {at}", role.label()),
            (Some(at), None) => format!("nothing in this plan uses a port at {at}"),
            (None, _) => "it fits".to_string(),
        },
        None => "it fits".to_string(),
        Some(s) if p.sides.len() > 1 => {
            format!("{} is {} short", s.role.label_for(s.switches), s.short)
        }
        Some(s) => format!("{} short on each switch", plural(s.short, "port")),
    };
    writeln!(w, "  {verdict} {} {why}", o.style.dim("-"))?;
    let width = b
        .sides
        .iter()
        .map(|s| s.role.label_for(s.switches).len())
        .max()
        .unwrap_or(0);
    for s in &b.sides {
        let used = match (s.port_speed, s.servers) {
            // Where the fabric and the servers both use ports of this
            // speed, the total alone hides which is which.
            (Some(sp), Some((servers, at))) if s.fabric > 0 && at == sp && b.speed.is_some() => {
                format!(
                    "{} of {} ports at {sp} ({} uplink, {servers} server)",
                    s.needed, b.ports, s.fabric
                )
            }
            // A switch whose ports are not all the same speed needs the
            // breakdown, or the total reads as a count of one kind of port.
            (Some(sp), Some((servers, at))) if b.speed.is_none() => format!(
                "{} of {} ports ({} at {sp}, {servers} at {at})",
                s.needed, b.ports, s.fabric
            ),
            (Some(sp), _) => format!("{} of {} ports at {sp}", s.needed, b.ports),
            (None, _) => format!("{} of {} ports", s.needed, b.ports),
        };
        let tail = if s.short > 0 {
            o.style.bad(&format!("{} short", s.short))
        } else {
            o.style.dim(&format!("{} spare", s.spare))
        };
        writeln!(
            w,
            "  {:<width$}  {used}, {tail}",
            s.role.label_for(s.switches)
        )?;
    }
    Ok(())
}

fn schedule(w: &mut impl Write, p: &Plan, o: &Opts) -> io::Result<()> {
    heading(
        w,
        o,
        &format!("Patch schedule for {}", plural(p.switches, "switch")),
    )?;
    field(
        w,
        o,
        "Notation",
        &if p.lanes > 1 {
            "sw<switch>:<port>/<lane>".to_string()
        } else {
            "sw<switch>:<port>".to_string()
        },
    )?;
    writeln!(w)?;

    let width = end_width(p);
    let limit = o.take();
    let mut shown = 0;
    let mut more = false;
    for patch in Schedule::new(p) {
        if shown == limit {
            more = true;
            break;
        }
        let a = format!("{:<width$}", patch.a.to_string());
        writeln!(
            w,
            "    {}  {}  {}",
            o.style.prefix(&a),
            o.style.dim("->"),
            o.style.prefix(&patch.b.to_string())
        )?;
        shown += 1;
    }
    if more {
        writeln!(
            w,
            "{}",
            o.style.dim(&format!(
                "    ... (showing {shown} of {}; drop -n for all of them)",
                num::group(&p.links.to_string())
            ))
        )?;
    }
    Ok(())
}

/// How wide the left-hand end of a patch line can get, worked out from the
/// plan rather than by looking at the lines - the schedule is an iterator, and
/// measuring it would mean building all of it first.
fn end_width(p: &Plan) -> usize {
    let ports = p.sides.iter().map(|s| s.ports).max().unwrap_or(1);
    let lane = if p.lanes > 1 {
        1 + p.lanes.to_string().len()
    } else {
        0
    };
    2 + p.switches.to_string().len() + 1 + ports.to_string().len() + lane
}

pub fn quiet(w: &mut impl Write, r: &Report, o: &Opts) -> io::Result<()> {
    // The schedule is what was asked for when it was asked for, and the bill
    // of materials is the useful output when it was not. Printing both would
    // put two shapes of line into one stream for no gain: a reader piping
    // this is after one of them.
    if r.schedule {
        for patch in Schedule::new(&r.plan).take(o.take()) {
            writeln!(w, "{} {}", patch.a, patch.b)?;
        }
        return Ok(());
    }
    for i in &r.plan.materials {
        writeln!(w, "{}\t{}", i.quantity, i.key)?;
    }
    Ok(())
}

pub fn json(w: &mut impl Write, r: &Report, o: &Opts) -> io::Result<()> {
    let p = &r.plan;
    let rate = |s: Option<Speed>| s.map_or(J::Null, |s| json::n(s.mbps()));
    let mut fields: Vec<(&'static str, J)> = vec![
        ("switches", json::n(p.switches)),
        ("spines", json::n(p.spines)),
        (
            "topology",
            json::s(match p.topology {
                Topology::Mesh => "mesh",
                Topology::Ring => "ring",
                Topology::Star => "star",
                Topology::LeafSpine => "leaf-spine",
            }),
        ),
        ("links", json::n(p.links)),
        ("links_per_pair", json::n(p.per_pair)),
        ("link_speed_mbps", rate(p.speed)),
        ("port_speed_mbps", rate(p.port_speed)),
        ("lanes_per_port", json::n(p.lanes)),
        ("media", json::s(p.media.name().to_lowercase())),
        (
            "arrangement",
            json::s(match p.arrangement {
                Arrangement::Straight => "straight",
                Arrangement::SplitBothEnds => "split-both-ends",
                Arrangement::SplitUpstream => "split-upstream",
            }),
        ),
        ("trunk_ports", json::n(p.trunk_ports)),
        ("spare_lanes", json::n(p.spare_lanes)),
        ("hops", json::n(p.hops)),
        ("resilience_links", json::n(p.resilience)),
        (
            "spine_loss",
            match p.spine_loss() {
                None => J::Null,
                Some(loss) => J::Obj(vec![
                    ("uplinks_per_leaf", json::n(loss.uplinks)),
                    ("uplinks_left", json::n(loss.left)),
                    (
                        "fabric_left_mbps",
                        p.speed.map_or(J::Null, |sp| json::n(sp.mbps() * loss.left)),
                    ),
                    (
                        "oversubscription_mbps",
                        match p.leaves().and_then(|l| l.ratio_over(p.speed, loss.left)) {
                            None => J::Null,
                            Some(r) => J::Obj(vec![
                                ("attached_mbps", json::n(r.down)),
                                ("fabric_mbps", json::n(r.up)),
                                ("blocking", J::Bool(r.blocking())),
                            ]),
                        },
                    ),
                ]),
            },
        ),
        (
            "switch_roles",
            J::Arr(
                p.sides
                    .iter()
                    .map(|s| {
                        J::Obj(vec![
                            ("role", json::s(s.role.key())),
                            ("switches", json::n(s.switches)),
                            ("links", json::n(s.degree)),
                            ("ports", json::n(s.ports)),
                            ("port_speed_mbps", rate(s.port_speed)),
                            ("lanes_per_port", json::n(s.lanes)),
                            ("spare_lanes", json::n(s.spare_lanes)),
                            (
                                "servers",
                                match s.access {
                                    None => J::Null,
                                    Some(a) => J::Obj(vec![
                                        ("ports", json::n(a.servers)),
                                        ("speed_mbps", json::n(a.speed.mbps())),
                                        ("switch_ports", json::n(a.ports)),
                                        ("switch_port_speed_mbps", json::n(a.port_speed.mbps())),
                                        ("lanes_per_port", json::n(a.lanes)),
                                        ("spare_lanes", json::n(a.spare_lanes)),
                                        ("attached_mbps", json::n(a.speed.mbps() * a.servers)),
                                    ]),
                                },
                            ),
                            (
                                "oversubscription",
                                match s.ratio(p.speed) {
                                    None => J::Null,
                                    Some(r) => J::Obj(vec![
                                        ("attached_mbps", json::n(r.down)),
                                        ("fabric_mbps", json::n(r.up)),
                                        ("blocking", J::Bool(r.blocking())),
                                    ]),
                                },
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "bandwidth",
            J::Obj(vec![
                ("bisection_links", json::n(p.bisection)),
                (
                    "bisection_mbps",
                    p.speed.map_or(J::Null, |s| json::n(s.mbps() * p.bisection)),
                ),
                (
                    "per_pair_mbps",
                    p.speed.map_or(J::Null, |s| json::n(s.mbps() * p.per_pair)),
                ),
                (
                    "total_mbps",
                    p.speed.map_or(J::Null, |s| json::n(s.mbps() * p.links)),
                ),
            ]),
        ),
        (
            "materials",
            J::Arr(p.materials.iter().map(material).collect()),
        ),
        ("cautions", J::Arr(p.cautions.iter().map(json::s).collect())),
    ];
    fields.push((
        "port_budgets",
        J::Arr(
            r.budgets
                .iter()
                .map(|b| {
                    J::Obj(vec![
                        ("ports", json::n(b.ports)),
                        ("speed_mbps", b.speed.map_or(J::Null, |s| json::n(s.mbps()))),
                        ("role", b.role.map_or(J::Null, |r| json::s(r.key()))),
                        ("fits", J::Bool(b.fits())),
                        (
                            "roles",
                            J::Arr(
                                b.sides
                                    .iter()
                                    .map(|s| {
                                        J::Obj(vec![
                                            ("role", json::s(s.role.key())),
                                            ("needed", json::n(s.needed)),
                                            ("spare", json::n(s.spare)),
                                            ("short", json::n(s.short)),
                                        ])
                                    })
                                    .collect(),
                            ),
                        ),
                    ])
                })
                .collect(),
        ),
    ));
    if r.schedule {
        let patches: Vec<_> = Schedule::new(p).take(o.take()).collect();
        fields.push((
            "schedule",
            J::Obj(vec![
                ("links", json::n(p.links)),
                ("listed", json::n(patches.len())),
                (
                    "patches",
                    J::Arr(
                        patches
                            .iter()
                            .map(|c| {
                                J::Obj(vec![
                                    ("a", json::s(c.a.to_string())),
                                    ("b", json::s(c.b.to_string())),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]),
        ));
    }
    write!(w, "{}", J::Obj(fields).render())
}

fn material(i: &Item) -> J {
    J::Obj(vec![
        ("item", json::s(i.key.clone())),
        ("quantity", json::n(i.quantity)),
        ("description", json::s(i.label.clone())),
        ("where", json::s(i.note.clone())),
    ])
}

fn plural(n: u64, what: &str) -> String {
    let plural = match what {
        "switch" => "switches".to_string(),
        "leaf" => "leaves".to_string(),
        w => format!("{w}s"),
    };
    format!(
        "{} {}",
        num::group(&n.to_string()),
        if n == 1 { what.to_string() } else { plural }
    )
}

/// A labelled value. The label is padded before it is styled, because escape
/// sequences have no width but would still be counted by the formatter.
fn field(w: &mut impl Write, o: &Opts, label: &str, value: &str) -> io::Result<()> {
    writeln!(w, "  {}{value}", o.style.dim(&format!("{label:<LABEL$}")))
}

/// A field whose value is one line per kind of switch, labelled once.
fn rows(w: &mut impl Write, o: &Opts, label: &str, values: Vec<String>) -> io::Result<()> {
    for (n, value) in values.iter().enumerate() {
        field(w, o, if n == 0 { label } else { "" }, value)?;
    }
    Ok(())
}

/// A section heading, always preceded by a blank line.
fn heading(w: &mut impl Write, o: &Opts, text: &str) -> io::Result<()> {
    writeln!(w, "\n{}", o.style.bold(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fabric::{Media, ops, plan};
    use crate::style::When;

    /// The shape still reads inline in these cases, as `/star`, so the lists
    /// below stay one line each; it is a flag on the command line now.
    fn report(switches: u64, args: &[&str], media: Media) -> Report {
        let mut opts = plan::Options {
            media,
            ..plan::Options::default()
        };
        let mut ops = Vec::new();
        for arg in args {
            match arg.strip_prefix("--shape=") {
                Some(shape) => {
                    opts.shape = Some(
                        <Topology as clap::ValueEnum>::from_str(&shape.to_ascii_lowercase(), true)
                            .expect("a shape"),
                    )
                }
                None if *arg == "--schedule" => opts.schedule = true,
                None => ops.push(ops::parse(arg).expect("an operator")),
            }
        }
        plan::build(switches, &ops, opts).expect("plans")
    }

    /// The default the binary uses: the whole schedule, however long it is.
    fn opts(style: Style) -> Opts {
        Opts { limit: None, style }
    }

    fn rendered(switches: u64, args: &[&str]) -> String {
        let r = report(switches, args, Media::Optic);
        let mut out = Vec::new();
        text(&mut out, &r, &opts(Style::plain())).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn quieted(switches: u64, args: &[&str]) -> String {
        let r = report(switches, args, Media::Optic);
        let mut out = Vec::new();
        quiet(&mut out, &r, &opts(Style::plain())).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        }
        out
    }

    /// The layout invariant is that stripping the escapes from a coloured
    /// report gives back the uncoloured one byte for byte. A value styled
    /// before it was padded silently shifts every column after it, and looks
    /// fine in a plain-text test.
    #[test]
    fn colour_never_changes_the_layout() {
        for (switches, args) in [
            (8u64, vec!["@100G", "%400G", "=32", "--schedule"]),
            (
                9,
                vec!["@25G", "%100G", "--shape=star", "=16", "--schedule"],
            ),
            (4, vec!["--shape=ring", "x2", "@400G", "=4", "--schedule"]),
            (2, vec![]),
            (16, vec!["@300G", "%3", "=8"]),
            (
                16,
                vec!["+2", "@100G", "%400G", "-48@25G", "=56", "--schedule"],
            ),
            (2, vec!["+2", "@100G", "-48@25G"]),
            (16, vec!["+1", "@100G", "-48@25G"]),
            (
                8,
                vec![
                    "+4", "@100G", "%400G", "-48@25G", "=32@400G", "=4@100G", "=2@200G",
                ],
            ),
            (16, vec!["+12", "@100G", "-48@25G%100G", "=64"]),
            (8, vec!["-24@10G"]),
        ] {
            let r = report(switches, &args, Media::Optic);
            let mut plain = Vec::new();
            text(&mut plain, &r, &opts(Style::plain())).unwrap();
            let mut painted = Vec::new();
            text(&mut painted, &r, &opts(Style::new(When::Always))).unwrap();
            let plain = String::from_utf8(plain).unwrap();
            let painted = String::from_utf8(painted).unwrap();
            assert!(
                painted.contains('\x1b'),
                "{switches} {args:?} was not painted"
            );
            assert_eq!(strip_ansi(&painted), plain, "{switches} {args:?}");
        }
    }

    #[test]
    fn the_report_opens_with_what_was_asked_about() {
        let s = rendered(8, &["@100G", "%400G"]);
        assert!(s.starts_with("8 switches  -  full mesh at 100G\n"), "{s}");
        assert!(s.contains("Links          28"), "{s}");
        assert!(s.contains("2 x 400G on each switch"), "{s}");
        assert!(s.contains("(4 lanes each: 7 used, 1 spare)"), "{s}");
        assert!(s.contains("Bisection      1.6T"), "{s}");
        assert!(s.contains("Fabric total   2.8T"), "{s}");
    }

    /// Two rows used to restate what the report had already said: the
    /// per-pair capacity when a pair has one link, and the per-switch link
    /// count when there was no speed to turn it into a capacity.
    #[test]
    fn the_report_does_not_say_the_same_thing_twice() {
        // One link to a pair, so the per-pair row would repeat the link speed.
        let s = rendered(8, &["@100G"]);
        assert!(s.contains("Link speed     100G"), "{s}");
        assert!(!s.contains("Per pair"), "{s}");
        // Two links to a pair is a sum worth showing.
        let s = rendered(8, &["@100G", "x2"]);
        assert!(s.contains("Per pair       200G  (2 x 100G)"), "{s}");

        // Without a speed the per-switch row is the port count again.
        let s = rendered(8, &[]);
        assert!(!s.contains("Per switch"), "{s}");
        assert!(s.contains("Ports          7 ports on each switch"), "{s}");
        // With one it carries the capacity, which the ports row does not.
        let s = rendered(8, &["@100G"]);
        assert!(s.contains("Per switch     7 links, 700G"), "{s}");
    }

    #[test]
    fn one_of_a_kind_of_switch_is_the_one_not_each() {
        let s = rendered(2, &["--shape=star", "@100G", "-24@10G", "=32:hub"]);
        assert!(s.contains("the hub 1 link, 100G"), "{s}");
        assert!(s.contains("the spoke 1 link, 100G"), "{s}");
        assert!(s.contains("1 x 100G on the spoke"), "{s}");
        assert!(s.contains("24 x 10G on the spoke"), "{s}");
        // The rows say "the spoke"; only the topology's own description of
        // the shape still talks about spokes in general.
        assert!(!s.contains("each spoke 1 link"), "{s}");
        assert!(!s.contains("on each spoke"), "{s}");
        // The budget rows follow the same rule.
        assert!(
            s.contains("the hub  1 of 32 ports at 100G, 31 spare"),
            "{s}"
        );
    }

    #[test]
    fn a_fabric_with_no_speeds_leaves_the_rates_out() {
        let s = rendered(8, &[]);
        assert!(s.starts_with("8 switches  -  full mesh\n"), "{s}");
        assert!(!s.contains("Link speed"), "{s}");
        assert!(!s.contains("Fabric total"), "{s}");
        assert!(s.contains("16 links across the middle"), "{s}");
    }

    #[test]
    fn a_star_reports_its_two_kinds_of_switch_separately() {
        let s = rendered(9, &["@25G", "%100G", "--shape=star"]);
        assert!(s.contains("the hub 8 links, 200G"), "{s}");
        assert!(s.contains("each spoke 1 link, 25G"), "{s}");
        assert!(s.contains("2 x 100G on the hub"), "{s}");
        assert!(s.contains("1 x 25G on each spoke"), "{s}");
    }

    #[test]
    fn the_verdict_on_a_port_budget_comes_first() {
        let s = rendered(8, &["@100G", "%400G", "=32"]);
        assert!(
            s.contains("Ports on a 32-port switch\n  yes - it fits"),
            "{s}"
        );
        let s = rendered(48, &["@100G", "--shape=star", "=32"]);
        assert!(
            s.contains("Ports on a 32-port switch\n  no - the hub is 15 short"),
            "{s}"
        );
        assert!(s.contains("47 of 32 ports at 100G, 15 short"), "{s}");
    }

    /// A schedule that was asked for comes out whole. Trimming it is what
    /// `-n` is for, and only then does it say it was trimmed.
    #[test]
    fn a_schedule_comes_out_whole_unless_it_was_trimmed() {
        let r = report(8, &["--schedule"], Media::Optic);
        let mut out = Vec::new();
        text(&mut out, &r, &opts(Style::plain())).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert_eq!(s.lines().filter(|l| l.contains("->")).count(), 28);
        assert!(!s.contains("showing"), "{s}");

        let mut out = Vec::new();
        let trimmed = Opts {
            limit: Some(3),
            style: Style::plain(),
        };
        text(&mut out, &r, &trimmed).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("sw1:1  ->  sw2:1"), "{s}");
        assert!(
            s.contains("... (showing 3 of 28; drop -n for all of them)"),
            "{s}"
        );
    }

    #[test]
    fn quiet_prints_the_bill_and_nothing_else() {
        let s = quieted(8, &["@100G", "%400G"]);
        assert_eq!(
            s,
            "16\ttransceiver-400G\n16\tbreakout-1x4-400G\n28\tcoupler-100G\n16\tport-switch-400G\n"
        );
        // The keys are the interface, so a leaf-spine's are spelled out too:
        // 32 links, two ends each, and 16 ports on each of two spines.
        let s = quieted(16, &["+2", "@100G", "-48@25G"]);
        assert_eq!(
            s,
            "64\ttransceiver-100G\n32\tpatch-lead\n32\tport-spine-100G\n\
             32\tport-leaf-100G\n768\taccess-port-leaf-25G\n"
        );
    }

    #[test]
    fn quiet_prints_the_schedule_when_the_schedule_was_asked_for() {
        let s = quieted(4, &["--schedule"]);
        assert_eq!(
            s,
            "sw1:1 sw2:1\nsw1:2 sw3:1\nsw1:3 sw4:1\nsw2:2 sw3:2\nsw2:3 sw4:2\nsw3:3 sw4:3\n"
        );
        // It carries no prose, no padding and no colour, because it is
        // somebody's input.
        assert!(!s.contains("->"));
    }

    #[test]
    fn a_leaf_spine_says_which_switches_are_which() {
        let s = rendered(16, &["+2", "@100G", "%400G", "-48@25G"]);
        assert!(
            s.starts_with("16 leaves + 2 spines  -  leaf-spine at 100G\n"),
            "{s}"
        );
        assert!(
            s.contains("Switches       18  (16 leaves, 2 spines)"),
            "{s}"
        );
        assert!(
            s.contains("Links          32  (1 link from each leaf to each spine)"),
            "{s}"
        );
        assert!(s.contains("4 x 400G on each spine"), "{s}");
        assert!(s.contains("2 x 100G on each leaf"), "{s}");
        assert!(s.contains("Server ports   48 x 25G on each leaf"), "{s}");
        assert!(s.contains("Spine ports    8"), "{s}");
        assert!(
            s.contains("each spine's 400G ports split 4 ways, a whole 100G port at each leaf"),
            "{s}"
        );
    }

    #[test]
    fn a_trunk_port_count_says_it_is_the_whole_fabric() {
        // The Ports line above it counts one switch's front panel, so this
        // one has to be readable as a total rather than as more of the same.
        let mesh = rendered(8, &["@100G", "%400G"]);
        assert!(
            mesh.contains("Trunk ports    16  (across 8 switches: 64 lanes, 56 used, 8 spare)"),
            "{mesh}"
        );
        let clos = rendered(8, &["+4", "@100G", "%400G"]);
        assert!(
            clos.contains("Spine ports    8  (across 4 spines: 32 lanes, 32 used, 0 spare)"),
            "{clos}"
        );
        // A hub is one switch, so there is nothing to spread the count over.
        let star = rendered(48, &["--shape=star", "@25G", "%100G"]);
        assert!(
            star.contains("Hub ports      12  (48 lanes, 47 used, 1 spare)"),
            "{star}"
        );
    }

    #[test]
    fn the_ratio_is_the_headline_of_its_own_section() {
        let s = rendered(16, &["+2", "@100G", "-48@25G"]);
        assert!(
            s.contains("Per leaf       6:1  (1.2T attached against 200G of fabric)"),
            "{s}"
        );
        assert!(
            s.contains("Servers        768 x 25G  (19.2T attached in total)"),
            "{s}"
        );
        // Enough uplink, and it says so rather than leaving the reader to
        // notice which side of 1:1 the number fell.
        let s = rendered(16, &["+12", "@100G", "-48@25G"]);
        assert!(
            s.contains("Per leaf       1:1  (1.2T attached against 1.2T of fabric - non-blocking)"),
            "{s}"
        );
        let s = rendered(16, &["+15", "@100G", "-48@25G"]);
        assert!(s.contains("Per leaf       1:1.25"), "{s}");
        // A ratio nobody can work out is not printed as though they could.
        let s = rendered(16, &["+2", "-48@25G"]);
        assert!(s.contains("attached, fabric unknown"), "{s}");
        // And a fabric with nothing attached has no section at all.
        assert!(!rendered(8, &["@100G"]).contains("Oversubscription"));
    }

    #[test]
    fn a_spine_pair_says_what_losing_one_costs() {
        let s = rendered(16, &["+2", "@100G", "-48@25G"]);
        assert!(
            s.contains("Spine loss     each leaf keeps 1 uplink of 2  (100G of 200G)"),
            "{s}"
        );
        assert!(
            s.contains("One spine down 12:1  (100G of fabric left on each leaf)"),
            "{s}"
        );
        // One spine has nothing to lose, and is a caution rather than a row.
        let s = rendered(16, &["+1", "@100G", "-48@25G"]);
        assert!(!s.contains("Spine loss"), "{s}");
        assert!(!s.contains("One spine down"), "{s}");
        assert!(
            s.contains("Caution        one spine is a single point of failure"),
            "{s}"
        );
        // And a shape without spines never mentions them.
        assert!(!rendered(8, &["@100G", "-24@10G"]).contains("Spine loss"));
    }

    #[test]
    fn a_budget_can_be_asked_about_one_kind_of_port() {
        let s = rendered(
            8,
            &["+4", "@100G", "%400G", "-48@25G", "=32@400G", "=4@100G"],
        );
        assert!(
            s.contains("400G ports on a 32-port switch\n  yes - it fits\n  each spine  2 of 32 ports at 400G, 30 spare"),
            "{s}"
        );
        assert!(s.contains("100G ports on a 4-port switch"), "{s}");
        assert!(
            s.contains("each leaf  4 of 4 ports at 100G, 0 spare"),
            "{s}"
        );

        // Four spines at 200G is four 200G ports on a leaf that has two.
        let s = rendered(8, &["+4", "@200G", "%400G", "-48@25G", "=2@200G"]);
        assert!(
            s.contains("200G ports on a 2-port switch\n  no - each leaf is 2 short"),
            "{s}"
        );
        // And a speed nothing runs at is said outright.
        let s = rendered(8, &["+4", "@100G", "-48@25G", "=32@400G"]);
        assert!(
            s.contains("yes - nothing in this plan uses a port at 400G"),
            "{s}"
        );
    }

    #[test]
    fn a_budget_that_names_a_switch_says_so_in_its_heading() {
        // Leaves and spines both at 100G, so the role is what separates them.
        let s = rendered(8, &["+2", "@100G", "=4@100G:leaf", "=4@100G:spine"]);
        assert!(
            s.contains(
                "100G ports on a 4-port leaf\n  yes - it fits\n  \
                 each leaf  2 of 4 ports at 100G, 2 spare"
            ),
            "{s}"
        );
        assert!(
            s.contains("100G ports on a 4-port spine\n  no - each spine is 4 short"),
            "{s}"
        );
        // A named role with nothing at that speed says which role, rather
        // than only that nothing does.
        let s = rendered(8, &["+2", "@100G", "=4@400G:leaf"]);
        assert!(s.contains("yes - each leaf uses no port at 400G"), "{s}");
    }

    #[test]
    fn a_rack_of_two_leaves_reaches_both_spines() {
        let s = rendered(2, &["+2", "@100G", "-48@25G"]);
        assert!(
            s.starts_with("2 leaves + 2 spines  -  leaf-spine at 100G\n"),
            "{s}"
        );
        assert!(
            s.contains("Links          4  (1 link from each leaf to each spine)"),
            "{s}"
        );
        assert!(s.contains("each leaf 2 links, 200G"), "{s}");
        assert!(s.contains("2 x 100G on each spine"), "{s}");
    }

    #[test]
    fn a_budget_breaks_down_a_switch_whose_ports_differ() {
        let s = rendered(16, &["+2", "@100G", "-48@25G", "=56"]);
        assert!(
            s.contains("each leaf   50 of 56 ports (2 at 100G, 48 at 25G)"),
            "{s}"
        );
        let s = rendered(16, &["+2", "@100G", "-48@25G", "=48"]);
        assert!(s.contains("no - each leaf is 2 short"), "{s}");
    }

    #[test]
    fn json_carries_exact_numbers() {
        let r = report(8, &["@100G", "%400G", "=32", "--schedule"], Media::Optic);
        let mut out = Vec::new();
        json(&mut out, &r, &opts(Style::plain())).unwrap();
        let s = String::from_utf8(out).unwrap();
        for want in [
            "\"links\": 28",
            "\"link_speed_mbps\": 100000",
            "\"port_speed_mbps\": 400000",
            "\"lanes_per_port\": 4",
            "\"arrangement\": \"split-both-ends\"",
            "\"trunk_ports\": 16",
            "\"spare_lanes\": 8",
            "\"bisection_mbps\": 1600000",
            "\"total_mbps\": 2800000",
            "\"fits\": true",
            "\"listed\": 28",
            "\"a\": \"sw1:1/1\"",
        ] {
            assert!(s.contains(want), "{want} missing from {s}");
        }
        // It is never coloured and never grouped, because it is parsed
        // rather than read.
        assert!(!s.contains('\x1b') && !s.contains("2,800"));
    }

    #[test]
    fn json_carries_the_servers_and_the_ratio() {
        let r = report(16, &["+2", "@100G", "%400G", "-48@25G"], Media::Optic);
        let mut out = Vec::new();
        json(&mut out, &r, &opts(Style::plain())).unwrap();
        let s = String::from_utf8(out).unwrap();
        for want in [
            "\"topology\": \"leaf-spine\"",
            "\"spines\": 2",
            "\"arrangement\": \"split-upstream\"",
            "\"role\": \"leaf\"",
            "\"ports\": 48",
            "\"switch_ports\": 48",
            "\"attached_mbps\": 1200000",
            "\"fabric_mbps\": 200000",
            "\"blocking\": true",
            "\"uplinks_per_leaf\": 2",
            "\"uplinks_left\": 1",
            "\"fabric_left_mbps\": 100000",
        ] {
            assert!(s.contains(want), "{want} missing from {s}");
        }
        // A spine has no servers, and says so rather than saying zero.
        assert!(s.contains("\"servers\": null"), "{s}");
    }
}
