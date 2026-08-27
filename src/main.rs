//! prefixtool - inspect, split and carve up IPv4/IPv6 prefixes.

mod carve;
mod info;
mod json;
mod num;
mod ops;
mod render;
mod report;
mod style;
mod wellknown;
mod zones;

use clap::Parser;
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

const AFTER_HELP: &str = "\
OPERATORS:
  /N            split the prefix into /N subnets
  %M            split it into M subnets, whatever lengths that needs
  %a:b:c        share it out in that ratio
  -N            carve one /N out of the prefix
  -N*K, -NxK    carve K subnets of /N (use the x form to keep zsh happy)
  -<prefix>     reserve one specific subnet, wherever it sits
  -N:name       any carve may be named; the name lands in the table and map
  +N            show the enclosing /N supernet
  +<prefix>     aggregate; several of these make one aggregate, not a pair each
  =<addr|net>   ask whether an address or prefix falls inside
  @N            the Nth subnet of a requested split; @-1 is the last
  ^N            the prefix N blocks along at the same size; ^-1 is previous
  .             the reverse DNS zones covering it; .N to pick the boundary

  Carve operators are pooled into a single allocation: fixed subnets are placed
  first, then floating ones best-fit from the lowest free block, or the highest
  with --from=top. A /N split or a %M given alongside a carve describes what
  the carve left over.

  A ratio is exact when its parts, reduced, add up to a power of two: 2:1:1 can
  be cut from a prefix, 2:1 cannot, and the report says which one you got.

EXAMPLES:
  prefixtool 2001:db8::/64
        what is this prefix, how big, what is it reserved for

  prefixtool 2001:db8::/52 /64
        how many /64s does it hold, and where do they start and end

  prefixtool 10.0.0.0/24 %5
        divide it between five teams, as evenly as the space allows

  prefixtool 2001:db8::/48 %2:1:1
        share it between three sites, the first getting twice the space

  prefixtool 10.0.0.0/22 .
        which reverse DNS zones do I have to go and create

  prefixtool 2001:db8::/52 -56 -64x2
        carve out a /56 and two /64s, and aggregate what remains

  prefixtool 10.0.0.0/16 -10.0.8.0/22 -24x4 /24
        reserve an existing subnet, take four more /24s, count what is left

  prefixtool 10.0.0.0/16 -24:dmz -22:wifi --from=top
        take infrastructure down from the top, named, so the map reads as a plan

  prefixtool 2001:db8::/52 /64 =2001:db8:0:3::5
        which /64 does that address land in

  prefixtool 2001:db8::/52 /64 @3
        the other direction: which /64 is number 3

  prefixtool 10.0.4.0/22 ^1
        what comes straight after this block

  prefixtool 10.0.0.0/24 +10.0.1.0/24
        can these two be aggregated, and does it waste anything

COLOUR:
  The report is coloured when it is going to a terminal, and never when it is
  piped, redirected, or emitted as --json or --quiet. NO_COLOR and TERM=dumb
  turn it off; --color=always forces it on, for piping into less -R.

EXIT STATUS:
  0  success
  1  bad prefix or operator
  3  a carve request could not be satisfied
  4  --quiet, and an =<addr> asked about is outside the prefix

  Under --quiet an =<addr> is a question, so its answer is the exit status:

      prefixtool 10.0.0.0/8 =10.1.2.3 -q > /dev/null || echo not ours

  Outside is 4 rather than 1 so that it stays distinct from bad input. A
  mistyped address is a different thing from a confident no, and a script
  checking for one should never be handed the other.
";

#[derive(Parser)]
#[command(
    name = "prefixtool",
    version,
    about = "Inspect, split and carve up IPv4 and IPv6 prefixes",
    after_help = AFTER_HELP,
    max_term_width = 96
)]
struct Cli {
    /// The prefix to work on, e.g. 2001:db8::/52 or 10.0.0.0/16
    #[arg(value_name = "PREFIX")]
    prefix: String,

    /// Operators: /N, -N, -N*K, -<prefix>, +N, =<addr>  (see below)
    #[arg(value_name = "OP", allow_hyphen_values = true)]
    ops: Vec<String>,

    /// Maximum prefixes to list per section
    #[arg(short = 'n', long, value_name = "N", default_value_t = 8)]
    limit: usize,

    /// List every prefix, however many there are
    #[arg(short, long)]
    all: bool,

    /// Print prefixes only, one per line, for piping
    #[arg(short, long)]
    quiet: bool,

    /// Emit a JSON object instead of a report
    #[arg(long)]
    json: bool,

    /// Which end floating carves fill from: bottom or top
    #[arg(long, value_name = "END", default_value_t = From::Bottom,
          value_enum, hide_default_value = true)]
    from: From,

    /// When to colour the report: auto, always or never
    #[arg(long, value_name = "WHEN", default_value_t = style::When::Auto,
          value_enum, hide_default_value = true)]
    color: style::When,
}

/// Which end of the prefix `-N` allocations are taken from.
///
/// A clap-facing mirror of `carve::Direction`, so that the allocator does not
/// have to derive `ValueEnum` and pull the CLI's vocabulary into itself.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum From {
    Bottom,
    Top,
}

impl std::fmt::Display for From {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            From::Bottom => "bottom",
            From::Top => "top",
        })
    }
}

impl std::convert::From<From> for carve::Direction {
    fn from(v: From) -> carve::Direction {
        match v {
            From::Bottom => carve::Direction::Bottom,
            From::Top => carve::Direction::Top,
        }
    }
}

/// Move operators behind a `--` so that flags and operators can be given in
/// any order: `prefixtool 10.0.0.0/16 -24x4 --json` reads naturally, but clap
/// would otherwise hand `--json` to the operator list.
fn arrange<I: IntoIterator<Item = String>>(args: I) -> Vec<String> {
    let mut head = Vec::new();
    let mut ops = Vec::new();
    let mut after_dashdash = false;
    for (i, arg) in args.into_iter().enumerate() {
        if i == 0 || after_dashdash {
            // argv[0], then anything the user themselves put after `--`.
            if i == 0 {
                head.push(arg)
            } else {
                ops.push(arg)
            }
        } else if arg == "--" {
            after_dashdash = true;
        } else if ops::looks_like_op(&arg) {
            ops.push(arg);
        } else {
            head.push(arg);
        }
    }
    head.push("--".into());
    head.extend(ops);
    head
}

fn main() -> ExitCode {
    let cli = Cli::parse_from(arrange(std::env::args()));
    match run(&cli) {
        Ok(code) => code,
        Err(e) => {
            let s = style::Style::for_stderr(cli.color);
            eprintln!("{} {e}", s.bad("prefixtool:"));
            ExitCode::from(1)
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode, String> {
    let net = ops::parse_net(&cli.prefix)?;
    let parsed = cli
        .ops
        .iter()
        .map(|o| ops::parse(o))
        .collect::<Result<Vec<_>, _>>()?;
    let report = report::build(&cli.prefix, net, &parsed, cli.from.into())?;

    let opts = render::Opts {
        limit: cli.limit,
        all: cli.all,
        // Machine-readable output is never coloured, whatever was asked for.
        style: if cli.json || cli.quiet {
            style::Style::plain()
        } else {
            style::Style::new(cli.color)
        },
    };
    let stdout = io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    let written = if cli.json {
        render::json(&mut w, &report, &opts)
    } else if cli.quiet {
        render::quiet(&mut w, &report, &opts)
    } else {
        render::text(&mut w, &report, &opts)
    }
    .and_then(|()| w.flush());

    if let Err(e) = written {
        // `prefixtool ... | head` is a normal way to use this.
        if e.kind() == io::ErrorKind::BrokenPipe {
            return Ok(ExitCode::SUCCESS);
        }
        return Err(e.to_string());
    }

    // Under --quiet the report is a machine's input, so an `=` lookup becomes
    // a predicate and its answer is the exit status. It gets a code of its
    // own rather than reusing 1, so that a script asking whether an address
    // is inside can never read a mistyped address as a confident no. Left
    // alone in the other modes, where the answer is on screen to be read.
    let outside = cli.quiet && report.lookups.iter().any(|l| !l.inside);
    let unsatisfied = report.carve.as_ref().is_some_and(|p| !p.all_granted());
    Ok(if unsatisfied {
        // A plan that could not be carried out outranks the answer to one of
        // the questions asked alongside it.
        ExitCode::from(3)
    } else if outside {
        ExitCode::from(4)
    } else {
        ExitCode::SUCCESS
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::parse_from(arrange(args.iter().map(|s| s.to_string())))
    }

    #[test]
    fn flags_may_follow_operators() {
        let cli = parse(&[
            "prefixtool",
            "2001:db8::/52",
            "-56",
            "-64x2",
            "--json",
            "-n",
            "2",
        ]);
        assert_eq!(cli.prefix, "2001:db8::/52");
        assert_eq!(cli.ops, vec!["-56", "-64x2"]);
        assert!(cli.json);
        assert_eq!(cli.limit, 2);
    }

    #[test]
    fn flags_may_precede_operators() {
        let cli = parse(&["prefixtool", "--all", "10.0.0.0/16", "/24"]);
        assert_eq!(cli.prefix, "10.0.0.0/16");
        assert_eq!(cli.ops, vec!["/24"]);
        assert!(cli.all);
    }

    #[test]
    fn an_explicit_dashdash_forces_operators() {
        let cli = parse(&["prefixtool", "10.0.0.0/16", "--", "-24"]);
        assert_eq!(cli.ops, vec!["-24"]);
    }

    #[test]
    fn a_bare_prefix_needs_no_operators() {
        let cli = parse(&["prefixtool", "2001:db8::/64"]);
        assert_eq!(cli.prefix, "2001:db8::/64");
        assert!(cli.ops.is_empty());
    }
}
