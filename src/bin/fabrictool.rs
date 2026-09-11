//! fabrictool - how many cables and transceivers it takes to wire a set of
//! switches to each other.

use clap::Parser;
use prefixtool::fabric::plan::Problem;
use prefixtool::fabric::{Media, ops, plan, render};
use prefixtool::style;
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

const AFTER_HELP: &str = "\
OPERATORS:
  @SPEED        the speed of each link: @100G, @25G, @1.6T
  %M            break each switch port into M lanes
  %SPEED        the same, worked out from the port speed instead of counted
  xK, *K        K parallel links between each pair (use the x form in zsh)
  /SHAPE        /mesh (the default), /ring, /star or /leaf-spine
  +S            S spines above the leaves, which makes it a leaf-spine
  -N@SPEED      N server ports on each leaf, at that speed
  -N@SPEED%P    the same, out of P ports split into lanes to reach them
  =N            each switch has N ports - does the plan fit?
  .             the patch schedule: which port on which switch reaches which

  A bare number after % is a lane count and a number with a unit is a port
  speed, so %4 and %400G are different questions and neither has to be
  guessed at. Both need a link speed before the bill of materials can name
  the parts, but neither needs one to count them.

  Where every switch is the same shape - a mesh or a ring - both ends of a
  link are lanes of a broken-out port, so they meet in a patch field and the
  optics sit at the trunk ports. Where they are not - a star's hub, a
  leaf-spine's spines - the upstream end breaks out and each switch below it
  takes a whole port, which is what a DAC or AOC splitter cable is built for
  and what a 400G spine port fanned out to four 100G leaves is.

  Server ports are the other half of an oversubscription ratio: what is
  attached to a switch against what leaves it. They land on whichever
  switches are the fabric's edge - the leaves of a leaf-spine, the spokes of
  a star, every switch of a mesh or a ring - and they count against a port
  budget alongside the fabric's own ports, because they come out of the same
  front panel.

EXAMPLES:
  fabrictool 8
        how many cables does a mesh of eight switches take

  fabrictool 8 @100G
        the same, with what it adds up to in bandwidth

  fabrictool 8 @100G %400G
        out of 400G ports, four 100G lanes each: how many optics is that

  fabrictool 16 @400G %800G =32
        a mesh of sixteen out of 800G ports - does it fit a 32-port switch

  fabrictool 4 @400G x2
        two links between each pair, so any one of them can fail

  fabrictool 24 /ring @100G
        a ring instead, and what that costs in bandwidth across the middle

  fabrictool 48 /star @25G %100G --media=dac
        one hub, 25G to each spoke, split four ways out of its 100G ports

  fabrictool 16 +2 @100G %400G -48@25G
        sixteen leaves under two spines, 400G spine ports fanned out to 100G
        uplinks, 48 servers at 25G under each leaf - and what that is
        oversubscribed by

  fabrictool 16 +4 @100G %400G -48@25G%100G =56
        the same with four spines and the servers arriving on split 100G
        ports, against the 56 ports a leaf actually has

  fabrictool 8 @100G %400G . --all
        the schedule to take to the rack, every link of it

COLOUR:
  The report is coloured when it is going to a terminal, and never when it is
  piped, redirected, or emitted as --json or --quiet. NO_COLOR and TERM=dumb
  turn it off; --color=always forces it on, for piping into less -R.

EXIT STATUS:
  0  success
  1  bad switch count or operator
  3  the fabric cannot be built as asked
  4  --quiet, and an =N port budget does not fit

  Under --quiet an =N is a question, so its answer is the exit status:

      fabrictool 32 @100G %400G =32 -q > /dev/null || echo needs bigger switches

  Not fitting is 4 rather than 1 so that it stays distinct from bad input: a
  mistyped port count is a different thing from a confident no.

  --quiet prints the bill of materials as 'quantity<TAB>item', one per line,
  or the patch schedule as one link per line when . was given.
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
    /// How many switches to connect - leaves, when +S puts spines above them
    // Taken as text and converted in run() rather than by clap, so that a
    // mistyped count leaves by the same exit code as a mistyped operator.
    #[arg(value_name = "SWITCHES")]
    switches: String,

    /// Operators: @SPEED, %M, xK, /mesh, +S, -N@SPEED, =N, .  (see below)
    #[arg(value_name = "OP", allow_hyphen_values = true)]
    ops: Vec<String>,

    /// Maximum links to list in the patch schedule
    #[arg(short = 'n', long, value_name = "N", default_value_t = 8)]
    limit: usize,

    /// List every link, however many there are
    #[arg(short, long)]
    all: bool,

    /// Print the bill of materials only, one item per line, for piping
    #[arg(short, long)]
    quiet: bool,

    /// Emit a JSON object instead of a report
    #[arg(long)]
    json: bool,

    /// What the links are made of: optic, aoc or dac
    #[arg(long, value_name = "KIND", default_value_t = Media::Optic,
          value_enum, hide_default_value = true)]
    media: Media,

    /// When to colour the report: auto, always or never
    #[arg(long, value_name = "WHEN", default_value_t = style::When::Auto,
          value_enum, hide_default_value = true)]
    color: style::When,
}

/// Move operators behind a `--` so that flags and operators can be given in
/// any order: `fabrictool 8 @100G --json` reads naturally, but clap would
/// otherwise hand `--json` to the operator list.
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
    let switches: u64 = cli
        .switches
        .parse()
        .map_err(|_| Problem::Input(format!("'{}' is not a number of switches", cli.switches)))?;
    let parsed = cli
        .ops
        .iter()
        .map(|o| ops::parse(o))
        .collect::<Result<Vec<_>, _>>()
        .map_err(Problem::Input)?;
    let report = plan::build(switches, &parsed, cli.media)?;

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
        // `fabrictool 256 . --all | head` is a normal way to use this.
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
        Cli::parse_from(arrange(args.iter().map(|s| s.to_string())))
    }

    #[test]
    fn flags_may_follow_operators() {
        let cli = parse(&["fabrictool", "8", "@100G", "%400G", "--json", "-n", "2"]);
        assert_eq!(cli.switches, "8");
        assert_eq!(cli.ops, vec!["@100G", "%400G"]);
        assert!(cli.json);
        assert_eq!(cli.limit, 2);
    }

    #[test]
    fn flags_may_precede_operators() {
        let cli = parse(&["fabrictool", "--all", "16", "/ring", "x2"]);
        assert_eq!(cli.switches, "16");
        assert_eq!(cli.ops, vec!["/ring", "x2"]);
        assert!(cli.all);
    }

    #[test]
    fn the_switch_count_is_not_mistaken_for_a_flag_value() {
        let cli = parse(&["fabrictool", "-n", "4", "8", "."]);
        assert_eq!((cli.switches.as_str(), cli.limit), ("8", 4));
        assert_eq!(cli.ops, vec!["."]);
    }

    #[test]
    fn an_explicit_dashdash_forces_operators() {
        let cli = parse(&["fabrictool", "8", "--", "x2"]);
        assert_eq!(cli.ops, vec!["x2"]);
    }

    #[test]
    fn a_bare_count_needs_no_operators() {
        let cli = parse(&["fabrictool", "4"]);
        assert_eq!(cli.switches, "4");
        assert!(cli.ops.is_empty());
    }
}
