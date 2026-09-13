//! fabrictool - how many cables and transceivers it takes to wire a set of
//! switches to each other.

use clap::Parser;
use prefixtool::fabric::plan::{Options, Problem};
use prefixtool::fabric::{dot, dsl, plan, render};
use prefixtool::style;
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

const AFTER_HELP: &str = "\
THE FABRIC:
  A fabric is a chain of tiers running from the core to the edge, and the
  thing between two tiers is the link that joins them. Three pieces of
  syntax carry all of it:

    N name        a tier: 8 leaves, 4 spines, 24 switches, 48 servers
    [...]         that switch's front panel: [32x400G], [48x25G,4x100G]
    -SPEED-       the links between the tiers either side of it: -100G-

  The tiers are spines, leaves, switches, hub, spokes and servers. Two
  tiers of switches make a leaf-spine, spines first; a hub over spokes
  makes a star. One tier says the pattern its own switches are wired in,
  as mesh or ring on the far side of its link. Servers, where there are
  any, are the tier the chain ends with.

  A link carries KxSPEED for parallel links between each pair, and :optic,
  :aoc or :dac for what it is made of. Write it as -- when no speed has
  been picked and only the cable count is wanted.

  A front panel entry is a count, a speed, or both: [32] is thirty-two
  ports of unstated speed, [400G] is ports at 400G however many it takes,
  and [32x400G] is both. Every entry with a count is a question the report
  answers, about the switch whose panel it is written on.

  Lanes are never written down. The port that carries a link is the
  smallest one at or above the link's speed, and where the port is the
  faster of the two it breaks out. A fabric with no speeds at all breaks
  nothing out and counts cables.

EXAMPLES:
  fabrictool '8 switches -- mesh'
        how many cables does a mesh of eight switches take

  fabrictool '8 switches -100G- mesh'
        the same, with what it adds up to in bandwidth

  fabrictool '8 switches[400G] -100G- mesh'
        out of 400G ports, four 100G lanes each: how many optics is that

  fabrictool '16 switches[32x800G] -400G- mesh'
        a mesh of sixteen out of 800G ports - does it fit a 32-port switch

  fabrictool '4 switches -2x400G- mesh'
        two links between each pair, so any one of them can fail

  fabrictool '24 switches -100G- ring'
        a ring instead, and what that costs in bandwidth

  fabrictool '1 hub[100G] -25G:dac- 47 spokes'
        one hub, 25G to each spoke, split four ways out of its 100G ports

  fabrictool '2 spines[400G] -100G- 16 leaves -25G- 48 servers'
        sixteen leaves under two spines, 400G spine ports fanned out to
        100G uplinks, 48 servers at 25G under each leaf - and what that is
        oversubscribed by

  fabrictool '4 spines[32x400G] -100G- 8 leaves[48x25G,2x200G,4x100G] -25G- 48 servers'
        the same checked against the front panel each switch really has

  fabrictool '8 switches[400G] -100G- mesh' --schedule
        the schedule to take to the rack, every link of it

  fabrictool '2 spines[400G] -100G- 16 leaves' --dot | dot -Tpng > fabric.png
        the same fabric as a picture, for checking the shape rather than
        counting it

COLOUR:
  The report is coloured when it is going to a terminal, and never when it is
  piped, redirected, or emitted as --json or --quiet. NO_COLOR and TERM=dumb
  turn it off; --color=always forces it on, for piping into less -R.

EXIT STATUS:
  0  success
  1  a fabric that does not read, or a count out of range
  3  the fabric cannot be built as asked
  4  --quiet, and a front panel does not fit

  Under --quiet a panel is a question, so its answer is the exit status:

      fabrictool '48 switches[32] -100G- mesh' -q > /dev/null || echo needs bigger switches

  Not fitting is 4 rather than 1 so that it stays distinct from bad input: a
  mistyped port count is a different thing from a confident no.

  --quiet prints the bill of materials as 'quantity<TAB>item', one per line,
  or the patch schedule as one link per line when --schedule was given.

  --dot emits a Graphviz graph: one edge per pair of switches, or one per
  cable with --schedule.
";

#[derive(Parser)]
#[command(
    name = "fabrictool",
    version,
    about = "Size the cables, transceivers and ports a set of switches needs",
    after_help = AFTER_HELP,
    max_term_width = 96
)]
struct Cli {
    /// The fabric: '4 spines[32x400G] -100G- 8 leaves -25G- 48 servers'
    // Taken as text and parsed in run(), so that a fabric that does not read
    // leaves by the same exit code as one that cannot be built.
    #[arg(value_name = "FABRIC", allow_hyphen_values = true)]
    fabric: String,

    /// Show only the first N links of the patch schedule
    #[arg(short = 'n', long, value_name = "N")]
    limit: Option<usize>,

    /// Print the bill of materials only, one item per line, for piping
    #[arg(short, long)]
    quiet: bool,

    /// Emit a JSON object instead of a report
    #[arg(long)]
    json: bool,

    /// Print the patch schedule: which port on which switch reaches which
    #[arg(long)]
    schedule: bool,

    /// Emit a Graphviz DOT graph of the fabric instead of a report
    #[arg(long)]
    dot: bool,

    /// When to colour the report: auto, always or never
    #[arg(long, value_name = "WHEN", default_value_t = style::When::Auto,
          value_enum, hide_default_value = true)]
    color: style::When,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(e) => {
            let s = style::Style::for_stderr(cli.color);
            eprintln!("{} {e}", s.bad("fabrictool:"));
            // A request that makes sense but cannot be built is a different
            // answer from one that does not parse, and a script telling them
            // apart should not have to read the message.
            ExitCode::from(match e {
                Problem::Impossible(_) => 3,
                Problem::Input(_) => 1,
            })
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode, Problem> {
    let fabric = dsl::parse(&cli.fabric).map_err(Problem::Input)?;
    let report = plan::build(
        fabric.switches,
        &fabric.ops,
        Options {
            media: fabric.media,
            shape: fabric.shape,
            schedule: cli.schedule,
        },
    )?;

    let opts = render::Opts {
        limit: cli.limit,
        // Machine-readable output is never coloured, whatever was asked for.
        style: if cli.json || cli.quiet || cli.dot {
            style::Style::plain()
        } else {
            style::Style::new(cli.color)
        },
    };
    let stdout = io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    let written = if cli.json {
        render::json(&mut w, &report, &opts)
    } else if cli.dot {
        dot::write(&mut w, &report)
    } else if cli.quiet {
        render::quiet(&mut w, &report, &opts)
    } else {
        render::text(&mut w, &report, &opts)
    }
    .and_then(|()| w.flush());

    if let Err(e) = written {
        // `fabrictool 256 --schedule | head` is a normal way to use this.
        if e.kind() == io::ErrorKind::BrokenPipe {
            return Ok(ExitCode::SUCCESS);
        }
        return Err(Problem::Input(e.to_string()));
    }

    // Under --quiet the report is a machine's input, so `=N` becomes a
    // predicate and its answer is the exit status. Left alone in the other
    // modes, where the answer is on screen to be read.
    let short = cli.quiet && report.budgets.iter().any(|b| !b.fits());
    Ok(if short {
        ExitCode::from(4)
    } else {
        ExitCode::SUCCESS
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::parse_from(args)
    }

    /// The fabric is one argument, so the flags sit either side of it and
    /// clap needs no help telling them apart.
    #[test]
    fn the_fabric_is_one_argument_among_the_flags() {
        let cli = parse(&[
            "fabrictool",
            "8 switches[400G] -100G- mesh",
            "--json",
            "-n",
            "2",
        ]);
        assert_eq!(cli.fabric, "8 switches[400G] -100G- mesh");
        assert!(cli.json);
        assert_eq!(cli.limit, Some(2));

        let cli = parse(&["fabrictool", "--json", "24 switches -100G- ring"]);
        assert_eq!(cli.fabric, "24 switches -100G- ring");
        assert!(cli.json);
    }

    /// A fabric starts with a digit, so it is never mistaken for a flag's
    /// value however the two are ordered.
    #[test]
    fn the_fabric_is_not_mistaken_for_a_flag_value() {
        let cli = parse(&["fabrictool", "-n", "4", "8 switches -- mesh", "--schedule"]);
        assert_eq!(
            (cli.fabric.as_str(), cli.limit),
            ("8 switches -- mesh", Some(4))
        );
        assert!(cli.schedule);
    }
}
