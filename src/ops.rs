//! Parsing for the little operator language that follows the prefix on the
//! command line.
//!
//! ```text
//!   /64        split the prefix into /64s
//!   -56        carve one /56 out of it
//!   -64*2      carve two /64s  (-64x2 is the same thing, and survives zsh)
//!   -10.0.1.0/24   carve out that exact subnet
//!   +48        show the enclosing /48
//!   +10.1.0.0/16   aggregate with that prefix
//!   =10.0.1.5  ask whether an address or prefix falls inside
//!   @3         the 3rd subnet of the requested split (@-1 is the last)
//!   ^1         the next prefix of the same size (^-1 is the previous)
//! ```

use ipnet::IpNet;
use nom::branch::alt;
use nom::bytes::complete::take_till;
use nom::character::complete::{char, one_of};
use nom::combinator::{all_consuming, map, map_opt, map_res, opt, rest};
use nom::error::{ErrorKind, FromExternalError, ParseError};
use nom::sequence::preceded;
use nom::{IResult, Parser};
use std::net::IpAddr;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// `/N` - divide into equal /N subnets.
    Split(u8),
    /// `-N` / `-N*K` - allocate `count` subnets of length `len`.
    Carve { len: u8, count: u64 },
    /// `-<prefix>` - remove one specific subnet.
    Exclude(IpNet),
    /// `+N` - the enclosing prefix of length N.
    Supernet(u8),
    /// `+<prefix>` - the smallest prefix holding both.
    Aggregate(IpNet),
    /// `=<addr|prefix>` - containment test.
    Contains(Target),
    /// `@N` - the Nth subnet of a requested split, counting from zero.
    /// Negative counts back from the end, so `@-1` is the last.
    Nth(i64),
    /// `^N` - the prefix N blocks along at the same length. Negative walks
    /// backwards, so `^-1` is the previous block.
    Step(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Addr(IpAddr),
    Net(IpNet),
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Addr(a) => write!(f, "{a}"),
            Target::Net(n) => write!(f, "{n}"),
        }
    }
}

impl Target {
    pub fn is_ipv4(&self) -> bool {
        match self {
            Target::Addr(a) => a.is_ipv4(),
            Target::Net(n) => n.addr().is_ipv4(),
        }
    }
}

/// A nom error that can carry one of our own messages.
///
/// nom's own errors say which combinator gave up and where, which is the wrong
/// vocabulary for a user who typed `-64*banana`. Leaf conversions therefore
/// fail with a written explanation, and this type carries it back out. A
/// branch that fails structurally carries nothing, so it never outranks a
/// branch that has something useful to say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason(Option<String>);

impl<I> ParseError<I> for Reason {
    fn from_error_kind(_: I, _: ErrorKind) -> Self {
        Reason(None)
    }

    fn append(_: I, _: ErrorKind, other: Self) -> Self {
        other
    }

    /// `alt` folds the branches' errors through this. A branch that explained
    /// itself beats one that only failed to match.
    fn or(self, other: Self) -> Self {
        if other.0.is_some() { other } else { self }
    }
}

impl<I, E: std::fmt::Display> FromExternalError<I, E> for Reason {
    fn from_external_error(_: I, _: ErrorKind, e: E) -> Self {
        Reason(Some(e.to_string()))
    }
}

type R<'a, T> = IResult<&'a str, T, Reason>;

/// Parse one command-line operator.
pub fn parse(token: &str) -> Result<Op, String> {
    if token.is_empty() {
        return Err("empty operator".to_string());
    }
    match all_consuming(operator).parse(token) {
        Ok((_, op)) => Ok(op),
        Err(nom::Err::Error(Reason(Some(why))) | nom::Err::Failure(Reason(Some(why)))) => Err(why),
        // Nothing matched the leading character, so the sigil itself is wrong.
        Err(_) => Err(format!(
            "unknown operator '{token}': expected /N, -N, -N*K, -<prefix>, +N or =<addr>"
        )),
    }
}

/// The grammar proper: a sigil, then a payload whose shape the sigil chooses.
///
/// `-` and `+` each accept either a prefix or a number. The prefix branch is
/// tried first and fails silently when the payload is not an address, which is
/// what lets `-64` fall through to the numeric branch while `-banana` still
/// reports that it is not a prefix length.
fn operator(input: &'_ str) -> R<'_, Op> {
    alt((
        preceded(char('/'), map(prefix_len, Op::Split)),
        preceded(char('-'), alt((map(network, Op::Exclude), carve))),
        preceded(
            char('+'),
            alt((map(network, Op::Aggregate), map(prefix_len, Op::Supernet))),
        ),
        preceded(char('='), map(target, Op::Contains)),
        preceded(char('@'), map(index("subnet index"), Op::Nth)),
        preceded(char('^'), map(index("step"), Op::Step)),
    ))
    .parse(input)
}

/// `N` or `/N`, where the whole remaining payload is the number. Taking the
/// rest rather than just the digits is what keeps `/banana` reportable.
fn prefix_len(input: &'_ str) -> R<'_, u8> {
    map_res(preceded(opt(char('/')), rest), to_prefix_len).parse(input)
}

/// `N`, `N*K` or `NxK`. The `x` form survives zsh, which globs the `*`.
fn carve(input: &'_ str) -> R<'_, Op> {
    map_res(
        preceded(
            opt(char('/')),
            (
                take_till(|c| c == '*' || c == 'x' || c == 'X'),
                opt(preceded(one_of("*xX"), rest)),
            ),
        ),
        |(len, count): (&str, Option<&str>)| -> Result<Op, String> {
            let len = to_prefix_len(len)?;
            let count = match count {
                None => 1,
                Some(c) => c
                    .parse::<u64>()
                    .map_err(|_| format!("'{c}' is not a subnet count"))?,
            };
            if count == 0 {
                return Err("a subnet count of 0 does nothing".into());
            }
            Ok(Op::Carve { len, count })
        },
    )
    .parse(input)
}

/// A prefix, or nothing. Deliberately silent on failure: this is one arm of an
/// `alt` whose other arm has the better complaint when the payload is a number.
fn network(input: &'_ str) -> R<'_, IpNet> {
    map_opt(rest, |s: &str| parse_net(s).ok()).parse(input)
}

/// An address keeps its identity as an address; anything else must be a prefix.
fn target(input: &'_ str) -> R<'_, Target> {
    map_res(rest, |s: &str| -> Result<Target, String> {
        if let Ok(addr) = IpAddr::from_str(s) {
            return Ok(Target::Addr(addr));
        }
        Ok(Target::Net(parse_net(s)?))
    })
    .parse(input)
}

/// A signed index. `+3` reads naturally beside `-3` but i64 will not parse the
/// plus, so it is stripped first.
fn index(what: &'static str) -> impl FnMut(&str) -> R<'_, i64> {
    move |input| {
        map_res(preceded(opt(char('+')), rest), move |s: &str| {
            s.parse::<i64>()
                .map_err(|_| format!("'{s}' is not a {what}"))
        })
        .parse(input)
    }
}

fn to_prefix_len(s: &str) -> Result<u8, String> {
    let n: u16 = s
        .parse()
        .map_err(|_| format!("'{s}' is not a prefix length"))?;
    if n > 128 {
        return Err(format!("prefix length /{n} is longer than an IPv6 address"));
    }
    Ok(n as u8)
}

/// A prefix given without a length is treated as a single host.
pub fn parse_net(s: &str) -> Result<IpNet, String> {
    if let Ok(net) = IpNet::from_str(s) {
        return Ok(net);
    }
    if let Ok(addr) = IpAddr::from_str(s) {
        let len = if addr.is_ipv4() { 32 } else { 128 };
        return Ok(IpNet::new(addr, len).expect("host length is always valid"));
    }
    Err(format!("'{s}' is not an IP prefix or address"))
}

/// Does this argument look like one of our operators rather than a clap flag?
///
/// Operators and short flags both start with `-`, so the two have to be told
/// apart before clap sees them: `-24` is a carve, `-n` is a flag. Anything
/// that opens with an operator sigil counts, so genuinely malformed operators
/// still reach `parse` and get a useful error rather than "unexpected
/// argument".
pub fn looks_like_op(token: &str) -> bool {
    let Some(rest) = token.strip_prefix(['/', '+', '=', '-', '@', '^']) else {
        return false;
    };
    if !token.starts_with('-') {
        return true;
    }
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    rest.starts_with(|c: char| c.is_ascii_digit()) || looks_like_address(rest)
}

fn looks_like_address(s: &str) -> bool {
    s.contains(':') || s.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Op {
        parse(s).unwrap()
    }

    #[test]
    fn splits() {
        assert_eq!(p("/64"), Op::Split(64));
        assert_eq!(p("/0"), Op::Split(0));
    }

    #[test]
    fn carves_with_and_without_counts() {
        assert_eq!(p("-56"), Op::Carve { len: 56, count: 1 });
        assert_eq!(p("-64*2"), Op::Carve { len: 64, count: 2 });
        assert_eq!(p("-64x2"), Op::Carve { len: 64, count: 2 });
        assert_eq!(p("-/24"), Op::Carve { len: 24, count: 1 });
    }

    #[test]
    fn carves_a_named_subnet() {
        assert_eq!(
            p("-10.0.1.0/24"),
            Op::Exclude("10.0.1.0/24".parse().unwrap())
        );
        assert_eq!(
            p("-2001:db8::/64"),
            Op::Exclude("2001:db8::/64".parse().unwrap())
        );
    }

    #[test]
    fn supernets_and_containment() {
        assert_eq!(p("+48"), Op::Supernet(48));
        assert_eq!(
            p("=10.0.1.5"),
            Op::Contains(Target::Addr("10.0.1.5".parse().unwrap()))
        );
        assert_eq!(
            p("=10.0.1.0/24"),
            Op::Contains(Target::Net("10.0.1.0/24".parse().unwrap()))
        );
    }

    #[test]
    fn plus_takes_either_a_length_or_a_prefix() {
        assert_eq!(p("+48"), Op::Supernet(48));
        assert_eq!(p("+/48"), Op::Supernet(48));
        assert_eq!(
            p("+10.1.0.0/16"),
            Op::Aggregate("10.1.0.0/16".parse().unwrap())
        );
        assert_eq!(
            p("+2001:db8:1::/48"),
            Op::Aggregate("2001:db8:1::/48".parse().unwrap())
        );
        // A bare address aggregates as a host route.
        assert_eq!(
            p("+10.1.2.3"),
            Op::Aggregate("10.1.2.3/32".parse().unwrap())
        );
    }

    #[test]
    fn nth_and_step_take_signed_indexes() {
        assert_eq!(p("@0"), Op::Nth(0));
        assert_eq!(p("@3"), Op::Nth(3));
        assert_eq!(p("@-1"), Op::Nth(-1));
        assert_eq!(p("^1"), Op::Step(1));
        assert_eq!(p("^+1"), Op::Step(1));
        assert_eq!(p("^-2"), Op::Step(-2));
        assert!(parse("@").is_err());
        assert!(parse("@two").is_err());
        assert!(parse("^one").is_err());
    }

    #[test]
    fn bare_address_becomes_a_host_route() {
        assert_eq!(parse_net("10.0.0.1").unwrap().prefix_len(), 32);
        assert_eq!(parse_net("2001:db8::1").unwrap().prefix_len(), 128);
    }

    #[test]
    fn operators_are_distinguishable_from_flags() {
        for op in [
            "/64",
            "-56",
            "-64*2",
            "-/24",
            "-10.0.0.0/24",
            "-2001:db8::/48",
            "+48",
            "=10.0.0.1",
        ] {
            assert!(looks_like_op(op), "{op} should look like an operator");
        }
        for flag in [
            "-n",
            "8",
            "--json",
            "--all",
            "-q",
            "-a",
            "--limit=4",
            "10.0.0.0/8",
        ] {
            assert!(
                !looks_like_op(flag),
                "{flag} should not look like an operator"
            );
        }
    }

    #[test]
    fn malformed_operators_still_reach_the_parser() {
        // So the user gets our error message, not clap's.
        assert!(looks_like_op("-64*banana"));
        assert!(parse("-64*banana").is_err());
    }

    #[test]
    fn a_branch_with_something_to_say_beats_one_that_merely_failed() {
        // `-banana` fails both arms of the alt: the prefix arm has nothing
        // useful to add, so the numeric arm's complaint is what surfaces.
        assert_eq!(
            parse("-banana").unwrap_err(),
            "'banana' is not a prefix length"
        );
        assert_eq!(
            parse("+banana").unwrap_err(),
            "'banana' is not a prefix length"
        );
        // And when the prefix arm is the one that should win, it does.
        assert_eq!(
            parse("-10.0.0.0/24"),
            Ok(Op::Exclude("10.0.0.0/24".parse().unwrap()))
        );
    }

    #[test]
    fn reason_prefers_an_explanation_over_silence() {
        use nom::error::ParseError;
        // The input type is only there to satisfy the trait; `or` ignores it.
        fn or(a: Reason, b: Reason) -> Reason {
            <Reason as ParseError<&str>>::or(a, b)
        }
        let silent = Reason(None);
        let spoke = Reason(Some("because".into()));
        assert_eq!(or(silent.clone(), spoke.clone()), spoke);
        assert_eq!(or(spoke.clone(), silent), spoke);
    }

    #[test]
    fn trailing_junk_is_rejected_rather_than_ignored() {
        // all_consuming: a parser that stopped early would silently accept
        // half an operator.
        assert!(parse("/64junk").is_err());
        assert!(parse("@1junk").is_err());
        assert!(parse("^1junk").is_err());
        assert!(parse("-64*2junk").is_err());
    }

    #[test]
    fn rejects_nonsense() {
        assert!(parse("64").is_err());
        assert!(parse("/129").is_err());
        assert!(parse("-64*0").is_err());
        assert!(parse("-64*banana").is_err());
        assert!(parse("").is_err());
    }
}
