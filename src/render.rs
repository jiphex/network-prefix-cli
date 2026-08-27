//! Output formatting: a human-readable report, a machine-readable one, and a
//! bare list of prefixes for piping into other tools.

use crate::carve::{Outcome, Plan};
use crate::info::{Info, Reverse};
use crate::json::{self, J};
use crate::num::{self, Count};
use crate::ops::Target;
use crate::report::{Report, Source, Split};
use crate::style::Style;
use crate::wellknown::Relation;
use ipnet::IpNet;
use std::io::{self, Write};

pub struct Opts {
    /// How many prefixes to list per section.
    pub limit: usize,
    /// List every prefix, however many there are.
    pub all: bool,
    /// Colour for the human-readable report. Machine output ignores it.
    pub style: Style,
}

impl Opts {
    fn take(&self) -> usize {
        if self.all { usize::MAX } else { self.limit }
    }
}

const LABEL: usize = 15;

pub fn text(w: &mut impl Write, r: &Report, o: &Opts) -> io::Result<()> {
    let i = &r.info;
    writeln!(
        w,
        "{}  {}  {}",
        o.style.title(&i.net.to_string()),
        o.style.dim("-"),
        i.family()
    )?;
    if let Some(given) = &i.given {
        writeln!(
            w,
            "{}",
            o.style
                .dim(&format!("  (given as {given}; host bits cleared)"))
        )?;
    }
    writeln!(w)?;

    field(w, o, "Network", &i.net.network().to_string())?;
    if i.is_ipv4() {
        field(w, o, "Broadcast", &i.last().to_string())?;
        field(w, o, "Netmask", &i.net.netmask().to_string())?;
        field(w, o, "Wildcard", &i.net.hostmask().to_string())?;
    } else {
        field(w, o, "Last address", &i.last().to_string())?;
        if let Some(e) = i.expanded() {
            field(w, o, "Expanded", &e)?;
        }
    }
    field(w, o, "Prefix length", &{
        let bits = i.net.max_prefix_len() - i.net.prefix_len();
        format!(
            "/{}  ({bits} host bit{})",
            i.net.prefix_len(),
            if bits == 1 { "" } else { "s" }
        )
    })?;
    field(w, o, "Addresses", &i.addresses.describe())?;
    if let Some(h) = &i.hosts {
        let mut v = format!(
            "{}  ({} - {})",
            num::group(&h.count.to_string()),
            h.first,
            h.last
        );
        if let Some(note) = h.note {
            v = format!("{}  [{}]", num::group(&h.count.to_string()), note);
        }
        field(w, o, "Usable hosts", &v)?;
    }

    let splits = i.common_splits();
    if !splits.is_empty() {
        let parts: Vec<String> = splits
            .iter()
            .filter_map(|len| {
                i.subnet_count(*len)
                    .map(|c| format!("{} x /{len}", c.grouped()))
            })
            .collect();
        field(w, o, "Holds", &parts.join("   "))?;
    }

    match &i.reverse {
        Reverse::Zone(z) => field(w, o, "Reverse DNS", z)?,
        Reverse::Unaligned(why) => field(w, o, "Reverse DNS", why)?,
    }

    for (n, m) in i.specials.iter().enumerate() {
        field(w, o, if n == 0 { "Ranges" } else { "" }, &m.describe())?;
    }
    for c in i.cautions() {
        field(
            w,
            o,
            "Caution",
            &o.style.warn(&format!(
                "{} is {} - not for general assignment",
                i.net, c.special.name
            )),
        )?;
    }

    for s in &r.supernets {
        heading(w, o, &format!("Supernet /{}", s.net.prefix_len()))?;
        writeln!(
            w,
            "  {}   holds {} x /{}",
            o.style.prefix(&s.net.to_string()),
            s.siblings.grouped(),
            i.net.prefix_len()
        )?;
    }

    for a in &r.aggregates {
        heading(w, o, &format!("Aggregate {} with {}", i.net, a.with))?;
        field(
            w,
            o,
            "Smallest",
            &format!("{}  ({})", a.net, size_hint(&a.net)),
        )?;
        if a.nested {
            let inner = if a.net == a.with { i.net } else { a.with };
            field(w, o, "Note", &format!("{} already contains {inner}", a.net))?;
        } else if a.exact {
            field(
                w,
                o,
                "Note",
                "exact - the two are siblings, nothing else is covered",
            )?;
        } else {
            field(
                w,
                o,
                "Also covers",
                &format!(
                    "{} block{} neither prefix uses",
                    a.spare.len(),
                    if a.spare.len() == 1 { "" } else { "s" }
                ),
            )?;
            list(w, a.spare.iter().copied(), o, "    ")?;
        }
    }

    for n in &r.neighbours {
        let sign = if n.step >= 0 { "+" } else { "" };
        heading(w, o, &format!("Step {sign}{}", n.step))?;
        writeln!(
            w,
            "  {}   {}",
            o.style.prefix(&n.net.to_string()),
            size_hint(&n.net)
        )?;
    }

    for p in &r.picks {
        heading(w, o, &format!("Subnet @{} of /{}", p.index, p.len))?;
        writeln!(
            w,
            "  {}   {}",
            o.style.prefix(&p.net.to_string()),
            o.style
                .dim(&format!("(index {})", num::group(&p.resolved.to_string())))
        )?;
    }

    for l in &r.lookups {
        heading(w, o, &format!("Lookup {}", l.target))?;
        if !l.inside {
            writeln!(
                w,
                "  {} {} is outside {}",
                o.style.bad("no -"),
                l.target,
                i.net
            )?;
            continue;
        }
        writeln!(w, "  {} inside {}", o.style.good("yes -"), i.net)?;
        for (len, sub, idx) in &l.positions {
            writeln!(
                w,
                "  /{len} -> {}   {}",
                o.style.prefix(&sub.to_string()),
                o.style
                    .dim(&format!("(subnet #{})", num::group(&idx.to_string())))
            )?;
        }
    }

    if let Some(plan) = &r.carve {
        carve_section(w, plan, o)?;
    }

    for s in &r.splits {
        split_section(w, &r.info, s, o)?;
    }
    Ok(())
}

fn carve_section(w: &mut impl Write, plan: &Plan, o: &Opts) -> io::Result<()> {
    heading(w, o, &format!("Carve from {}", plan.parent))?;
    let req = plan
        .grants
        .iter()
        .map(|g| g.label.len())
        .max()
        .unwrap_or(7)
        .max(7);
    let width = plan
        .grants
        .iter()
        .filter_map(|g| match &g.outcome {
            Outcome::Granted(n) => Some(n.to_string().len()),
            _ => None,
        })
        .max()
        .unwrap_or(10)
        .max(8);
    writeln!(
        w,
        "  {}",
        o.style.dim(&format!(
            "{:<req$}  {:<width$}  Size",
            "Request", "Assigned"
        ))
    )?;
    for g in &plan.grants {
        match &g.outcome {
            // Padded before styling: escape sequences have no width, but the
            // formatter counts them anyway and would skew every column.
            Outcome::Granted(n) => writeln!(
                w,
                "  {:<req$}  {}  {}",
                g.label,
                o.style.good(&format!("{:<width$}", n.to_string())),
                size_hint(n)
            )?,
            Outcome::Exhausted => writeln!(
                w,
                "  {:<req$}  {}  {}",
                g.label,
                o.style.bad(&format!("{:<width$}", "-")),
                o.style.bad(&format!("no space left in {}", plan.parent))
            )?,
            Outcome::Impossible(why) => writeln!(
                w,
                "  {:<req$}  {}  {}",
                g.label,
                o.style.bad(&format!("{:<width$}", "-")),
                o.style.bad(why)
            )?,
        }
    }

    writeln!(w)?;
    if plan.free.is_empty() {
        writeln!(
            w,
            "  Remaining      nothing - {} is fully allocated",
            plan.parent
        )?;
        return map_section(w, plan, o);
    }
    let counts = plan.free_counts();
    writeln!(
        w,
        "  Remaining      {} address{} in {} block{}",
        num::sum_grouped(&counts),
        if counts.len() == 1 && counts[0].as_u128() == Some(1) {
            ""
        } else {
            "es"
        },
        plan.free.len(),
        if plan.free.len() == 1 { "" } else { "s" }
    )?;
    if let Some(largest) = plan.largest_free() {
        writeln!(w, "  Largest block  {largest}  ({})", size_hint(&largest))?;
    }
    map_section(w, plan, o)
}

/// How many blocks either side of an allocation are worth showing for context.
const CONTEXT: usize = 3;

/// The parent laid out block by block, with the allocations marked in place.
///
/// Two flat lists - what was assigned, and what is free - leave the reader to
/// reconstruct the layout by comparing addresses. This shows it directly.
fn map_section(w: &mut impl Write, plan: &Plan, o: &Opts) -> io::Result<()> {
    let rows = plan.map();
    // Keep every allocation, plus its immediate neighbours for orientation.
    let carved: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.carved)
        .map(|(i, _)| i)
        .collect();
    // With nothing carved the map is just the free list again, and the
    // remaining-space summary above has already said everything there is.
    if carved.is_empty() {
        return Ok(());
    }
    heading(w, o, &format!("Map of {}", plan.parent))?;
    let mut keep: Vec<bool> = (0..rows.len())
        .map(|i| {
            if o.all {
                true
            } else {
                carved.iter().any(|c| i.abs_diff(*c) <= CONTEXT)
            }
        })
        .collect();
    // A run of one is not worth hiding: the line saying so is longer than the
    // line it replaces.
    for i in 0..keep.len() {
        let alone = !keep[i] && (i == 0 || keep[i - 1]) && (i + 1 == keep.len() || keep[i + 1]);
        if alone {
            keep[i] = true;
        }
    }

    let width = rows
        .iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, r)| r.net.to_string().len())
        .max()
        .unwrap_or(0);

    let mut elided: Vec<IpNet> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        if !keep[i] {
            elided.push(row.net);
            continue;
        }
        flush_elided(w, o, &mut elided)?;
        if row.carved {
            // Padded before styling: escape sequences would otherwise be
            // counted as width and pull the label out of line. Only the
            // carved rows are padded, so free rows carry no trailing space.
            writeln!(
                w,
                "  {} {}   {}",
                o.style.good("->"),
                o.style.good(&format!("{:<width$}", row.net.to_string())),
                o.style.dim("carved")
            )?;
        } else {
            writeln!(w, "     {}", o.style.prefix(&row.net.to_string()))?;
        }
    }
    flush_elided(w, o, &mut elided)
}

/// Collapse a run of hidden blocks into one line that still accounts for them.
fn flush_elided(w: &mut impl Write, o: &Opts, elided: &mut Vec<IpNet>) -> io::Result<()> {
    if elided.is_empty() {
        return Ok(());
    }
    let counts: Vec<Count> = elided
        .iter()
        .map(|n| Count::pow2(u32::from(n.max_prefix_len() - n.prefix_len())))
        .collect();
    let line = format!(
        "     ... {} block{}, {} addresses (use --all)",
        elided.len(),
        if elided.len() == 1 { "" } else { "s" },
        num::sum_grouped(&counts)
    );
    elided.clear();
    writeln!(w, "{}", o.style.dim(&line))
}

fn split_section(w: &mut impl Write, info: &Info, s: &Split, o: &Opts) -> io::Result<()> {
    match s.source {
        Source::Whole => heading(w, o, &format!("Split {} into /{}", info.net, s.len))?,
        Source::Remainder => heading(
            w,
            o,
            &format!(
                "Split the remaining space into /{} ({} free block{})",
                s.len,
                s.blocks.len(),
                if s.blocks.len() == 1 { "" } else { "s" }
            ),
        )?,
    }
    if s.blocks.is_empty() {
        writeln!(w, "  nothing left is big enough to hold a /{}", s.len)?;
        return Ok(());
    }
    let counts = s.counts();
    field(w, o, "Subnets", &num::sum_grouped(&counts))?;
    if let (Some(first), Some(last)) = (s.first(), s.last()) {
        field(w, o, "First", &first.to_string())?;
        field(w, o, "Last", &last.to_string())?;
    }
    let each = size_hint(&first_of(s));
    if !matches!(each.as_str(), "1 x /64" | "1 address") {
        field(w, o, "Each holds", &each)?;
    }
    if s.too_small > 0 {
        field(
            w,
            o,
            "Skipped",
            &format!(
                "{} free block{} smaller than a /{}",
                s.too_small,
                if s.too_small == 1 { "" } else { "s" },
                s.len
            ),
        )?;
    }
    writeln!(w)?;
    let shown = list(w, s.subnets(), o, "    ")?;
    let total = num::sum_grouped(&counts);
    if !o.all && shown == o.take() {
        writeln!(
            w,
            "{}",
            o.style.dim(&format!(
                "    ... (showing {shown} of {total}; use --all or -n N)"
            ))
        )?;
    }
    Ok(())
}

fn first_of(s: &Split) -> IpNet {
    s.first().expect("blocks is non-empty")
}

/// Write up to the configured number of prefixes, returning how many went out.
fn list(
    w: &mut impl Write,
    items: impl Iterator<Item = IpNet>,
    o: &Opts,
    indent: &str,
) -> io::Result<usize> {
    let mut n = 0;
    for item in items.take(o.take()) {
        writeln!(w, "{indent}{}", o.style.prefix(&item.to_string()))?;
        n += 1;
    }
    Ok(n)
}

/// A labelled value. The label is padded before it is styled, because escape
/// sequences have no width but would still be counted by the formatter.
fn field(w: &mut impl Write, o: &Opts, label: &str, value: &str) -> io::Result<()> {
    writeln!(w, "  {}{value}", o.style.dim(&format!("{label:<LABEL$}")))
}

/// A section heading, always preceded by a blank line.
fn heading(w: &mut impl Write, o: &Opts, text: &str) -> io::Result<()> {
    writeln!(w, "\n{}", o.style.bold(text))
}

/// A short description of how much space a prefix represents, in the units the
/// reader is most likely thinking in.
fn size_hint(net: &IpNet) -> String {
    let host_bits = u32::from(net.max_prefix_len() - net.prefix_len());
    if net.addr().is_ipv4() {
        let count = Count::pow2(host_bits);
        return match net.prefix_len() {
            32 => "1 address".into(),
            31 => "2 addresses".into(),
            _ => format!(
                "{} addresses, {} usable",
                count.grouped(),
                num::group(&(count.as_u128().unwrap_or(0) - 2).to_string())
            ),
        };
    }
    match net.prefix_len() {
        128 => "1 address".into(),
        len if len < 64 => format!("{} x /64", Count::pow2(u32::from(64 - len)).grouped()),
        64 => "1 x /64".into(),
        _ => format!("{} addresses", Count::pow2(host_bits).describe()),
    }
}

/// Just the prefixes, one per line, for `| xargs` and friends.
pub fn quiet(w: &mut impl Write, r: &Report, o: &Opts) -> io::Result<()> {
    for s in &r.supernets {
        writeln!(w, "{}", s.net)?;
    }
    for a in &r.aggregates {
        writeln!(w, "{}", a.net)?;
    }
    for n in &r.neighbours {
        writeln!(w, "{}", n.net)?;
    }
    for p in &r.picks {
        writeln!(w, "{}", p.net)?;
    }
    for l in &r.lookups {
        for (_, sub, _) in &l.positions {
            writeln!(w, "{sub}")?;
        }
    }
    if let Some(plan) = &r.carve {
        for n in plan.granted() {
            writeln!(w, "{n}")?;
        }
        if r.splits.is_empty() {
            list(w, plan.free.iter().copied(), o, "")?;
        }
    }
    for s in &r.splits {
        list(w, s.subnets(), o, "")?;
    }
    // With no operators at all, the prefix itself is the useful output.
    if r.supernets.is_empty()
        && r.aggregates.is_empty()
        && r.neighbours.is_empty()
        && r.picks.is_empty()
        && r.lookups.is_empty()
        && r.carve.is_none()
        && r.splits.is_empty()
    {
        writeln!(w, "{}", r.info.net)?;
    }
    Ok(())
}

pub fn json(w: &mut impl Write, r: &Report, o: &Opts) -> io::Result<()> {
    let i = &r.info;
    let mut fields: Vec<(&'static str, J)> = vec![
        ("prefix", json::s(i.net.to_string())),
        ("family", json::s(i.family())),
        ("network", json::s(i.net.network().to_string())),
        ("last_address", json::s(i.last().to_string())),
        ("prefix_length", json::n(i.net.prefix_len())),
        ("netmask", json::s(i.net.netmask().to_string())),
        ("hostmask", json::s(i.net.hostmask().to_string())),
        ("addresses", J::Num(i.addresses.digits())),
        (
            "host_bits",
            json::n(i.net.max_prefix_len() - i.net.prefix_len()),
        ),
    ];
    if let Some(e) = i.expanded() {
        fields.push(("expanded", json::s(e)));
    }
    fields.push((
        "usable_hosts",
        match &i.hosts {
            Some(h) => J::Obj(vec![
                ("count", json::n(h.count)),
                ("first", json::s(h.first.to_string())),
                ("last", json::s(h.last.to_string())),
                ("note", h.note.map_or(J::Null, json::s)),
            ]),
            None => J::Null,
        },
    ));
    fields.push((
        "reverse_dns",
        match &i.reverse {
            Reverse::Zone(z) => json::s(z.clone()),
            Reverse::Unaligned(_) => J::Null,
        },
    ));
    fields.push((
        "special_ranges",
        J::Arr(
            i.specials
                .iter()
                .map(|m| {
                    J::Obj(vec![
                        ("prefix", json::s(m.net.to_string())),
                        ("name", json::s(m.special.name)),
                        ("rfc", json::s(m.special.rfc)),
                        (
                            "relation",
                            json::s(match m.relation {
                                Relation::Within => "within",
                                Relation::Contains => "contains",
                                Relation::Overlaps => "overlaps",
                            }),
                        ),
                        ("caution", J::Bool(m.special.caution)),
                    ])
                })
                .collect(),
        ),
    ));

    if !r.supernets.is_empty() {
        fields.push((
            "supernets",
            J::Arr(
                r.supernets
                    .iter()
                    .map(|s| {
                        J::Obj(vec![
                            ("prefix", json::s(s.net.to_string())),
                            ("prefix_length", json::n(s.net.prefix_len())),
                            ("siblings", J::Num(s.siblings.digits())),
                        ])
                    })
                    .collect(),
            ),
        ));
    }

    if !r.lookups.is_empty() {
        fields.push((
            "lookups",
            J::Arr(
                r.lookups
                    .iter()
                    .map(|l| {
                        J::Obj(vec![
                            ("target", json::s(l.target.to_string())),
                            (
                                "target_kind",
                                json::s(match l.target {
                                    Target::Addr(_) => "address",
                                    Target::Net(_) => "prefix",
                                }),
                            ),
                            ("inside", J::Bool(l.inside)),
                            (
                                "positions",
                                J::Arr(
                                    l.positions
                                        .iter()
                                        .map(|(len, sub, idx)| {
                                            J::Obj(vec![
                                                ("prefix_length", json::n(len)),
                                                ("subnet", json::s(sub.to_string())),
                                                ("index", json::n(idx)),
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
    }

    if !r.aggregates.is_empty() {
        fields.push((
            "aggregates",
            J::Arr(
                r.aggregates
                    .iter()
                    .map(|a| {
                        J::Obj(vec![
                            ("with", json::s(a.with.to_string())),
                            ("prefix", json::s(a.net.to_string())),
                            ("prefix_length", json::n(a.net.prefix_len())),
                            ("exact", J::Bool(a.exact)),
                            ("nested", J::Bool(a.nested)),
                            (
                                "spare",
                                J::Arr(a.spare.iter().map(|n| json::s(n.to_string())).collect()),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ));
    }

    if !r.neighbours.is_empty() {
        fields.push((
            "neighbours",
            J::Arr(
                r.neighbours
                    .iter()
                    .map(|n| {
                        J::Obj(vec![
                            ("step", json::n(n.step)),
                            ("prefix", json::s(n.net.to_string())),
                        ])
                    })
                    .collect(),
            ),
        ));
    }

    if !r.picks.is_empty() {
        fields.push((
            "picks",
            J::Arr(
                r.picks
                    .iter()
                    .map(|p| {
                        J::Obj(vec![
                            ("index", json::n(p.index)),
                            ("resolved_index", json::n(p.resolved)),
                            ("prefix_length", json::n(p.len)),
                            ("subnet", json::s(p.net.to_string())),
                        ])
                    })
                    .collect(),
            ),
        ));
    }

    if let Some(plan) = &r.carve {
        fields.push((
            "carve",
            J::Obj(vec![
                ("parent", json::s(plan.parent.to_string())),
                ("satisfied", J::Bool(plan.all_granted())),
                (
                    "requests",
                    J::Arr(
                        plan.grants
                            .iter()
                            .map(|g| {
                                let (status, assigned, reason) = match &g.outcome {
                                    Outcome::Granted(n) => {
                                        ("granted", json::s(n.to_string()), J::Null)
                                    }
                                    Outcome::Exhausted => {
                                        ("exhausted", J::Null, json::s("no space left"))
                                    }
                                    Outcome::Impossible(why) => {
                                        ("impossible", J::Null, json::s(why.clone()))
                                    }
                                };
                                J::Obj(vec![
                                    ("request", json::s(g.label.clone())),
                                    ("status", json::s(status)),
                                    ("assigned", assigned),
                                    ("reason", reason),
                                ])
                            })
                            .collect(),
                    ),
                ),
                (
                    "free",
                    J::Arr(plan.free.iter().map(|n| json::s(n.to_string())).collect()),
                ),
                (
                    "free_addresses",
                    J::Num(num::sum_grouped(&plan.free_counts()).replace(',', "")),
                ),
                (
                    "largest_free_block",
                    plan.largest_free()
                        .map_or(J::Null, |n| json::s(n.to_string())),
                ),
                (
                    "map",
                    J::Arr(
                        plan.map()
                            .iter()
                            .map(|r| {
                                J::Obj(vec![
                                    ("prefix", json::s(r.net.to_string())),
                                    ("carved", J::Bool(r.carved)),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]),
        ));
    }

    if !r.splits.is_empty() {
        fields.push((
            "splits",
            J::Arr(
                r.splits
                    .iter()
                    .map(|s| {
                        let total = num::sum_grouped(&s.counts()).replace(',', "");
                        let subnets: Vec<IpNet> = s.subnets().take(o.take()).collect();
                        J::Obj(vec![
                            ("prefix_length", json::n(s.len)),
                            (
                                "source",
                                json::s(match s.source {
                                    Source::Whole => "prefix",
                                    Source::Remainder => "remaining_space",
                                }),
                            ),
                            ("count", J::Num(total)),
                            (
                                "first",
                                s.first().map_or(J::Null, |n| json::s(n.to_string())),
                            ),
                            ("last", s.last().map_or(J::Null, |n| json::s(n.to_string()))),
                            ("listed", json::n(subnets.len())),
                            (
                                "subnets",
                                J::Arr(subnets.iter().map(|n| json::s(n.to_string())).collect()),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ));
    }

    write!(w, "{}", J::Obj(fields).render())
}
