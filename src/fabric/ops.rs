//! The little operator language that follows the switch count on the command
//! line.
//!
//! ```text
//!   @100G     the speed of each link
//!   %4        break each switch port into 4 lanes
//!   %400G     the same, worked out from the port speed instead
//!   x2        two parallel links between each pair (*2 is the same thing)
//!   /ring     the shape: /mesh, /ring or /star
//!   =32       each switch has 32 ports - does this fit?
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
    /// `=32` - how many ports each switch has, as a question.
    Budget(u32),
    /// `.` - which port on which switch reaches which.
    Schedule,
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
        '=' => Ok(Op::Budget(budget(payload)?)),
        '.' if payload.is_empty() => Ok(Op::Schedule),
        '.' => Err(format!(
            "'.' takes nothing after it; '{token}' looks like something else"
        )),
        _ => Err(format!(
            "unknown operator '{token}': expected @SPEED, %M, xK, /mesh, =N or ."
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

fn budget(payload: &str) -> Result<u32, String> {
    let n: u32 = payload
        .parse()
        .map_err(|_| format!("'{payload}' is not a port count: write it like =32"))?;
    if n == 0 {
        return Err("a switch with 0 ports cannot be wired to anything".into());
    }
    Ok(n)
}

/// Whether a bare argument is an operator rather than a flag or the switch
/// count, so the two can be interleaved on the command line.
///
/// `x2` is the one that does not start with a sigil, and the digit after it
/// is what keeps it apart from a word that happens to begin with an x.
pub fn looks_like_op(token: &str) -> bool {
    if token.starts_with(['@', '%', '/', '=']) || token == "." {
        return true;
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
    fn reads_the_rest_of_the_operators() {
        assert_eq!(p("x2"), Op::PerPair(2));
        assert_eq!(p("X2"), Op::PerPair(2));
        assert_eq!(p("*2"), Op::PerPair(2));
        assert_eq!(p("/ring"), Op::Shape(Topology::Ring));
        assert_eq!(p("/MESH"), Op::Shape(Topology::Mesh));
        assert_eq!(p("=32"), Op::Budget(32));
        assert_eq!(p("."), Op::Schedule);
    }

    #[test]
    fn rejects_what_cannot_be_meant() {
        for bad in [
            "", "@", "@banana", "%", "%0", "%1", "%999", "%banana", "x0", "x", "xb", "/banana",
            "/", "=0", "=lots", ".5", "banana",
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
    }

    #[test]
    fn operators_are_told_apart_from_flags_and_counts() {
        for op in ["@100G", "%4", "%400G", "x2", "*2", "/ring", "=32", "."] {
            assert!(looks_like_op(op), "{op} should look like an operator");
        }
        for other in ["8", "16", "-n", "2", "--json", "--all", "-q", "xeon", "x"] {
            assert!(!looks_like_op(other), "{other} should not");
        }
    }
}
