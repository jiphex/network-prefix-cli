//! Parsing for the little operator language that follows the prefix on the
//! command line.
//!
//! ```text
//!   /64        split the prefix into /64s
//!   -56        carve one /56 out of it
//!   -64*2      carve two /64s  (-64x2 is the same thing, and survives zsh)
//!   -10.0.1.0/24   carve out that exact subnet
//!   +48        show the enclosing /48
//!   =10.0.1.5  ask whether an address or prefix falls inside
//! ```

use ipnet::IpNet;
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
    /// `=<addr|prefix>` - containment test.
    Contains(Target),
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

/// Parse one command-line operator.
pub fn parse(token: &str) -> Result<Op, String> {
    let (sigil, rest) = token
        .split_at_checked(1)
        .ok_or_else(|| "empty operator".to_string())?;
    // `-/56` reads naturally and means the same as `-56`.
    let rest = rest.strip_prefix('/').unwrap_or(rest);

    match sigil {
        "/" => Ok(Op::Split(prefix_len(rest)?)),
        "+" => Ok(Op::Supernet(prefix_len(rest)?)),
        "=" => Ok(Op::Contains(target(rest)?)),
        "-" => {
            if looks_like_address(rest) {
                let net = parse_net(rest)?;
                Ok(Op::Exclude(net))
            } else {
                let (len_part, count_part) = match rest.find(['*', 'x', 'X']) {
                    Some(i) => (&rest[..i], Some(&rest[i + 1..])),
                    None => (rest, None),
                };
                let len = prefix_len(len_part)?;
                let count = match count_part {
                    None => 1,
                    Some(c) => c
                        .parse::<u64>()
                        .map_err(|_| format!("'{c}' is not a subnet count"))?,
                };
                if count == 0 {
                    return Err("a subnet count of 0 does nothing".into());
                }
                Ok(Op::Carve { len, count })
            }
        }
        _ => Err(format!(
            "unknown operator '{token}': expected /N, -N, -N*K, -<prefix>, +N or =<addr>"
        )),
    }
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

fn target(s: &str) -> Result<Target, String> {
    if let Ok(addr) = IpAddr::from_str(s) {
        return Ok(Target::Addr(addr));
    }
    Ok(Target::Net(parse_net(s)?))
}

fn prefix_len(s: &str) -> Result<u8, String> {
    let n: u16 = s
        .parse()
        .map_err(|_| format!("'{s}' is not a prefix length"))?;
    if n > 128 {
        return Err(format!("prefix length /{n} is longer than an IPv6 address"));
    }
    Ok(n as u8)
}

/// Does this argument look like one of our operators rather than a clap flag?
///
/// Operators and short flags both start with `-`, so the two have to be told
/// apart before clap sees them: `-24` is a carve, `-n` is a flag. Anything
/// that opens with an operator sigil counts, so genuinely malformed operators
/// still reach `parse` and get a useful error rather than "unexpected
/// argument".
pub fn looks_like_op(token: &str) -> bool {
    let Some(rest) = token.strip_prefix(['/', '+', '=', '-']) else {
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
    fn rejects_nonsense() {
        assert!(parse("64").is_err());
        assert!(parse("/129").is_err());
        assert!(parse("-64*0").is_err());
        assert!(parse("-64*banana").is_err());
        assert!(parse("").is_err());
    }
}
