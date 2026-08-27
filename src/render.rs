//! Output formatting: a human-readable report, a machine-readable one, and a
//! bare list of prefixes for piping into other tools.

use crate::carve::{Outcome, Plan};
use crate::info::{Info, Reverse};
use crate::json::{self, J};
use crate::num::{self, Count};
use crate::ops::Target;
use crate::report::{Report, Source, Split};
use crate::wellknown::Relation;
use ipnet::IpNet;
use std::io::{self, Write};

pub struct Opts {
    /// How many prefixes to list per section.
    pub limit: usize,
    /// List every prefix, however many there are.
    pub all: bool,
}

impl Opts {
    fn take(&self) -> usize {
        if self.all { usize::MAX } else { self.limit }
    }
}

const LABEL: usize = 15;

pub fn text(w: &mut impl Write, r: &Report, o: &Opts) -> io::Result<()> {
    let i = &r.info;
    writeln!(w, "{}  -  {}", i.net, i.family())?;
    if let Some(given) = &i.given {
        writeln!(w, "  (given as {given}; host bits cleared)")?;
    }
    writeln!(w)?;

    field(w, "Network", &i.net.network().to_string())?;
    if i.is_ipv4() {
        field(w, "Broadcast", &i.last().to_string())?;
        field(w, "Netmask", &i.net.netmask().to_string())?;
        field(w, "Wildcard", &i.net.hostmask().to_string())?;
    } else {
        field(w, "Last address", &i.last().to_string())?;
        if let Some(e) = i.expanded() {
            field(w, "Expanded", &e)?;
        }
    }
    field(w, "Prefix length", &{
        let bits = i.net.max_prefix_len() - i.net.prefix_len();
        format!(
            "/{}  ({bits} host bit{})",
            i.net.prefix_len(),
            if bits == 1 { "" } else { "s" }
        )
    })?;
    field(w, "Addresses", &i.addresses.describe())?;
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
        field(w, "Usable hosts", &v)?;
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
        field(w, "Holds", &parts.join("   "))?;
    }

    match &i.reverse {
        Reverse::Zone(z) => field(w, "Reverse DNS", z)?,
        Reverse::Unaligned(why) => field(w, "Reverse DNS", why)?,
    }

    for (n, m) in i.specials.iter().enumerate() {
        field(w, if n == 0 { "Ranges" } else { "" }, &m.describe())?;
    }
    for c in i.cautions() {
        field(
            w,
            "Caution",
            &format!(
                "{} is {} - not for general assignment",
                i.net, c.special.name
            ),
        )?;
    }

    for s in &r.supernets {
        writeln!(w, "\nSupernet /{}", s.net.prefix_len())?;
        writeln!(
            w,
            "  {}   holds {} x /{}",
            s.net,
            s.siblings.grouped(),
            i.net.prefix_len()
        )?;
    }

    for l in &r.lookups {
        writeln!(w, "\nLookup {}", l.target)?;
        if !l.inside {
            writeln!(w, "  no - {} is outside {}", l.target, i.net)?;
            continue;
        }
        writeln!(w, "  yes - inside {}", i.net)?;
        for (len, sub, idx) in &l.positions {
            writeln!(
                w,
                "  /{len} -> {sub}   (subnet #{})",
                num::group(&idx.to_string())
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
    writeln!(w, "\nCarve from {}", plan.parent)?;
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
    writeln!(w, "  {:<req$}  {:<width$}  Size", "Request", "Assigned")?;
    for g in &plan.grants {
        match &g.outcome {
            Outcome::Granted(n) => writeln!(
                w,
                "  {:<req$}  {:<width$}  {}",
                g.label,
                n.to_string(),
                size_hint(n)
            )?,
            Outcome::Exhausted => writeln!(
                w,
                "  {:<req$}  {:<width$}  no space left in {}",
                g.label, "-", plan.parent
            )?,
            Outcome::Impossible(why) => {
                writeln!(w, "  {:<req$}  {:<width$}  {}", g.label, "-", why)?
            }
        }
    }

    writeln!(w)?;
    if plan.free.is_empty() {
        writeln!(
            w,
            "  Remaining      nothing - {} is fully allocated",
            plan.parent
        )?;
        return Ok(());
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
    let shown = list(w, plan.free.iter().copied(), o, "    ")?;
    if shown < plan.free.len() {
        writeln!(
            w,
            "    ... {} more free block{} (use --all or -n N)",
            plan.free.len() - shown,
            if plan.free.len() - shown == 1 {
                ""
            } else {
                "s"
            }
        )?;
    }
    Ok(())
}

fn split_section(w: &mut impl Write, info: &Info, s: &Split, o: &Opts) -> io::Result<()> {
    match s.source {
        Source::Whole => writeln!(w, "\nSplit {} into /{}", info.net, s.len)?,
        Source::Remainder => writeln!(
            w,
            "\nSplit the remaining space into /{} ({} free block{})",
            s.len,
            s.blocks.len(),
            if s.blocks.len() == 1 { "" } else { "s" }
        )?,
    }
    if s.blocks.is_empty() {
        writeln!(w, "  nothing left is big enough to hold a /{}", s.len)?;
        return Ok(());
    }
    let counts = s.counts();
    field(w, "Subnets", &num::sum_grouped(&counts))?;
    if let (Some(first), Some(last)) = (s.first(), s.last()) {
        field(w, "First", &first.to_string())?;
        field(w, "Last", &last.to_string())?;
    }
    let each = size_hint(&first_of(s));
    if !matches!(each.as_str(), "1 x /64" | "1 address") {
        field(w, "Each holds", &each)?;
    }
    if s.too_small > 0 {
        field(
            w,
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
        writeln!(w, "    ... (showing {shown} of {total}; use --all or -n N)")?;
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
        writeln!(w, "{indent}{item}")?;
        n += 1;
    }
    Ok(n)
}

fn field(w: &mut impl Write, label: &str, value: &str) -> io::Result<()> {
    writeln!(w, "  {label:<LABEL$}{value}")
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
    if r.supernets.is_empty() && r.lookups.is_empty() && r.carve.is_none() && r.splits.is_empty() {
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
