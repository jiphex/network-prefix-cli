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

use clap::Parser;
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

const AFTER_HELP: &str = "\
OPERATORS:
  /N            split the prefix into /N subnets
  -N            carve one /N out of the prefix
  -N*K, -NxK    carve K subnets of /N (use the x form to keep zsh happy)
  -<prefix>     reserve one specific subnet, wherever it sits
  +N            show the enclosing /N supernet
  +<prefix>     aggregate; several of these make one aggregate, not a pair each
  =<addr|net>   ask whether an address or prefix falls inside
  @N            the Nth subnet of a requested split; @-1 is the last
  ^N            the prefix N blocks along at the same size; ^-1 is previous

  Carve operators are pooled into a single allocation: fixed subnets are placed
  first, then floating ones best-fit from the lowest free block. A /N split
  given alongside a carve describes what the carve left over.

EXAMPLES:
  prefixtool 2001:db8::/64
        what is this prefix, how big, what is it reserved for

  prefixtool 2001:db8::/52 /64
        how many /64s does it hold, and where do they start and end

  prefixtool 2001:db8::/52 -56 -64x2
        carve out a /56 and two /64s, and aggregate what remains

  prefixtool 10.0.0.0/16 -10.0.8.0/22 -24x4 /24
        reserve an existing subnet, take four more /24s, count what is left

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

    /// When to colour the report: auto, always or never
    #[arg(long, value_name = "WHEN", default_value_t = style::When::Auto,
          value_enum, hide_default_value = true)]
    color: style::When,
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
    let report = report::build(&cli.prefix, net, &parsed)?;

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

    let unsatisfied = report.carve.as_ref().is_some_and(|p| !p.all_granted());
    Ok(if unsatisfied {
        ExitCode::from(3)
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
