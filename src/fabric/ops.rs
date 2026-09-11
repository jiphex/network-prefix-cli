//! The little operator language that follows the switch count on the command
//! line.
//!
//! ```text
//!   @100G     the speed of each link
//!   %4        break each switch port into 4 lanes
//!   %400G     the same, worked out from the port speed instead
//!   x2        two parallel links between each pair (*2 is the same thing)
//!   /ring     the shape: /mesh, /ring, /star or /leaf-spine
//!   +2        two spines above the leaves, which makes it a leaf-spine
//!   -48@25G   48 server ports on each leaf, at 25G
//!   -48@25G%100G  the same, out of 100G ports split four ways
//!   =32       each switch has 32 ports - does this fit?
//!   =4@100G   the same, about the four ports it has at one speed
//!   .         the patch schedule, port by port
//! ```
//!
//! Unlike the prefix side of this repository, the grammar here is a sigil and
//! a payload with no ambiguity to resolve between them, so it is a match on
//! the first character rather than a nom parser. The one decision worth
//! naming is what follows `%`: a bare number is a lane count and a number
//! with a unit is a port speed, which is why `%4` and `%400G` mean different
//! things and neither has to be guessed at.

use super::Topology;
use super::speed::{self, Speed};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// `@100G` - the speed of one link.
    Speed(Speed),
    /// `%4` or `%400G` - split each port into lanes.
    Breakout(Breakout),
    /// `x2` - how many parallel links join each pair of switches.
    PerPair(u32),
    /// `/ring` - the shape of the fabric.
    Shape(Topology),
    /// `=32`, or `=4@100G` - how many ports each switch has, as a question.
    /// A speed narrows it to the ports at that speed, which is the only way
    /// to ask about a switch whose front panel is not all one thing.
    Budget(Budget),
    /// `+2` - how many spines sit above the leaves. Saying so at all is what
    /// makes the shape a leaf-spine.
    Spines(u32),
    /// `-48@25G` - the ports on each leaf that face servers rather than the
    /// fabric. They are what an oversubscription ratio is a ratio of.
    Access(Access),
    /// `.` - which port on which switch reaches which.
    Schedule,
}

/// A port count to check the plan against, and what kind of port it counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub ports: u32,
    /// The speed those ports run at, when the question is about one kind of
    /// port rather than the whole panel.
    pub speed: Option<Speed>,
}

/// Ports facing whatever hangs off the fabric, as the user wrote them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    pub ports: u32,
    pub speed: Speed,
    /// How those ports are broken out, when they are: a 100G port is four
    /// 25G servers as easily as a 400G port is four 100G leaves.
    pub breakout: Option<Breakout>,
}

/// How a port is broken out, as the user said it rather than as it resolves:
/// a lane count stands on its own, a port speed needs the link speed before
/// it means anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breakout {
    Lanes(u32),
    Port(Speed),
}

/// The most lanes a single port is ever split into. Well beyond 1.6T split
/// into sixteen 100G lanes, and low enough that a mistyped speed does not
/// turn into a plan for an implausible harness.
const MAX_LANES: u32 = 64;

/// Parse one command-line operator.
pub fn parse(token: &str) -> Result<Op, String> {
    let Some(sigil) = token.chars().next() else {
        return Err("empty operator".into());
    };
    let payload = &token[sigil.len_utf8()..];
    match sigil {
        '@' => Ok(Op::Speed(speed::parse(payload)?)),
        '%' => Ok(Op::Breakout(breakout(payload)?)),
        'x' | 'X' | '*' => Ok(Op::PerPair(per_pair(payload)?)),
        '/' => Topology::parse(payload)
            .map(Op::Shape)
            .ok_or_else(|| format!("'{payload}' is not a topology: it is /mesh, /ring or /star")),
        '+' => Ok(Op::Spines(spines(payload)?)),
        '-' => Ok(Op::Access(access(payload)?)),
        '=' => Ok(Op::Budget(budget(payload)?)),
        '.' if payload.is_empty() => Ok(Op::Schedule),
        '.' => Err(format!(
            "'.' takes nothing after it; '{token}' looks like something else"
        )),
        _ => Err(format!(
            "unknown operator '{token}': expected @SPEED, %M, xK, /mesh, +S, -N@SPEED, =N or ."
        )),
    }
}

/// `%4` is four lanes to a port; `%400G` is a 400G port, and how many lanes
/// that is depends on the link speed, which may not have been read yet.
fn breakout(payload: &str) -> Result<Breakout, String> {
    if payload.is_empty() {
        return Err("missing a breakout: write it like %4 or %400G".into());
    }
    if payload.bytes().all(|b| b.is_ascii_digit()) {
        let lanes: u32 = payload
            .parse()
            .map_err(|_| format!("'{payload}' is not a lane count"))?;
        return match lanes {
            0 => Err("a port cannot be split into 0 lanes".into()),
            1 => Err("splitting a port into 1 lane is what not splitting it does".into()),
            n if n > MAX_LANES => Err(format!("{n} lanes to a port is more than any optic does")),
            n => Ok(Breakout::Lanes(n)),
        };
    }
    Ok(Breakout::Port(speed::parse(payload)?))
}

fn per_pair(payload: &str) -> Result<u32, String> {
    let n: u32 = payload
        .parse()
        .map_err(|_| format!("'{payload}' is not a number of links: write it like x2"))?;
    if n == 0 {
        return Err("0 links between each pair is not a fabric".into());
    }
    Ok(n)
}

/// `+2`, the spines above the leaves.
fn spines(payload: &str) -> Result<u32, String> {
    let n: u32 = payload
        .parse()
        .map_err(|_| format!("'{payload}' is not a number of spines: write it like +2"))?;
    if n == 0 {
        return Err("a leaf-spine with no spines is a pile of leaves".into());
    }
    Ok(n)
}

/// `-48@25G`, optionally `%100G` or `%4` for a port that is split to reach
/// them. The count and the speed are both required: a port count on its own
/// says nothing about bandwidth, which is the whole reason these are counted.
fn access(payload: &str) -> Result<Access, String> {
    let Some((count, rest)) = payload.split_once('@') else {
        return Err(format!(
            "'-{payload}' needs a speed as well as a count: write it like -48@25G"
        ));
    };
    let ports: u32 = count
        .parse()
        .map_err(|_| format!("'{count}' is not a number of ports: write it like -48@25G"))?;
    if ports == 0 {
        return Err("0 server ports is nothing hanging off it".into());
    }
    let (speed, breakout) = match rest.split_once('%') {
        Some((speed, split)) => (speed, Some(breakout(split)?)),
        None => (rest, None),
    };
    Ok(Access {
        ports,
        speed: speed::parse(speed)?,
        breakout,
    })
}

/// `=32`, or `=4@100G` for the ports at one speed.
fn budget(payload: &str) -> Result<Budget, String> {
    let (count, speed) = match payload.split_once('@') {
        Some((count, speed)) => (count, Some(speed::parse(speed)?)),
        None => (payload, None),
    };
    let ports: u32 = count
        .parse()
        .map_err(|_| format!("'{count}' is not a port count: write it like =32 or =4@100G"))?;
    if ports == 0 {
        return Err("a switch with 0 ports cannot be wired to anything".into());
    }
    Ok(Budget { ports, speed })
}

/// Whether a bare argument is an operator rather than a flag or the switch
/// count, so the two can be interleaved on the command line.
///
/// `x2` is the one that does not start with a sigil, and the digit after it
/// is what keeps it apart from a word that happens to begin with an x.
pub fn looks_like_op(token: &str) -> bool {
    if token.starts_with(['@', '%', '/', '=', '+']) || token == "." {
        return true;
    }
    // `-48@25G` is an operator and `-n` is a flag: what separates them is the
    // digit, exactly as it does on the prefix side.
    if let Some(rest) = token.strip_prefix('-') {
        return rest.starts_with(|c: char| c.is_ascii_digit());
    }
    match token.strip_prefix(['x', 'X', '*']) {
        Some(rest) => rest.starts_with(|c: char| c.is_ascii_digit()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Op {
        parse(s).expect("parses")
    }

    fn mbps(s: &str) -> u64 {
        speed::parse(s).unwrap().mbps()
    }

    #[test]
    fn reads_a_link_speed() {
        assert_eq!(p("@100G"), Op::Speed(speed::parse("100G").unwrap()));
        assert_eq!(p("@25g"), Op::Speed(speed::parse("25G").unwrap()));
    }

    #[test]
    fn a_bare_number_after_a_percent_is_lanes_and_a_unit_makes_it_a_speed() {
        assert_eq!(p("%4"), Op::Breakout(Breakout::Lanes(4)));
        match p("%400G") {
            Op::Breakout(Breakout::Port(s)) => assert_eq!(s.mbps(), mbps("400G")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn reads_spines_and_server_ports() {
        assert_eq!(p("+2"), Op::Spines(2));
        assert_eq!(
            p("-48@25G"),
            Op::Access(Access {
                ports: 48,
                speed: speed::parse("25G").unwrap(),
                breakout: None
            })
        );
        assert_eq!(
            p("-48@25G%100G"),
            Op::Access(Access {
                ports: 48,
                speed: speed::parse("25G").unwrap(),
                breakout: Some(Breakout::Port(speed::parse("100G").unwrap())),
            })
        );
        assert_eq!(
            p("-24@10G%4"),
            Op::Access(Access {
                ports: 24,
                speed: speed::parse("10G").unwrap(),
                breakout: Some(Breakout::Lanes(4)),
            })
        );
    }

    #[test]
    fn reads_the_rest_of_the_operators() {
        assert_eq!(p("x2"), Op::PerPair(2));
        assert_eq!(p("X2"), Op::PerPair(2));
        assert_eq!(p("*2"), Op::PerPair(2));
        assert_eq!(p("/ring"), Op::Shape(Topology::Ring));
        assert_eq!(p("/MESH"), Op::Shape(Topology::Mesh));
        assert_eq!(
            p("=32"),
            Op::Budget(Budget {
                ports: 32,
                speed: None
            })
        );
        assert_eq!(
            p("=4@100G"),
            Op::Budget(Budget {
                ports: 4,
                speed: Some(speed::parse("100G").unwrap()),
            })
        );
        assert_eq!(p("."), Op::Schedule);
    }

    #[test]
    fn rejects_what_cannot_be_meant() {
        for bad in [
            "",
            "@",
            "@banana",
            "%",
            "%0",
            "%1",
            "%999",
            "%banana",
            "x0",
            "x",
            "xb",
            "/banana",
            "/",
            "=0",
            "=lots",
            ".5",
            "banana",
            "+0",
            "+lots",
            "+",
            "-48",
            "-@25G",
            "-0@25G",
            "-48@",
            "-48@banana",
            "-lots@25G",
            "=0@100G",
            "=4@banana",
            "=lots@100G",
            "-48@25G%0",
            "-48@25G%banana",
        ] {
            assert!(parse(bad).is_err(), "{bad} should not parse");
        }
    }

    #[test]
    fn an_error_says_what_was_wrong_with_it() {
        assert!(parse("/banana").unwrap_err().contains("/mesh"));
        assert!(parse("%1").unwrap_err().contains("not splitting"));
        assert!(parse("@banana").unwrap_err().contains("100G"));
        assert!(parse("banana").unwrap_err().contains("unknown operator"));
        assert!(parse("-48").unwrap_err().contains("-48@25G"));
        assert!(parse("+0").unwrap_err().contains("pile of leaves"));
    }

    #[test]
    fn operators_are_told_apart_from_flags_and_counts() {
        for op in [
            "@100G", "%4", "%400G", "x2", "*2", "/ring", "=32", ".", "+2", "-48@25G",
        ] {
            assert!(looks_like_op(op), "{op} should look like an operator");
        }
        for other in [
            "8",
            "16",
            "-n",
            "2",
            "--json",
            "--all",
            "-q",
            "xeon",
            "x",
            "--media=dac",
        ] {
            assert!(!looks_like_op(other), "{other} should not");
        }
    }
}
