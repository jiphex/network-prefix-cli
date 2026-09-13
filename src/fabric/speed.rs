//! Port and link rates.
//!
//! Held in megabits per second, as a `u64`, because that is the coarsest unit
//! every rate anyone writes down divides into exactly: 2.5G is 2,500 and
//! 1.6T is 1,600,000. Floating point would put rounding error into counts
//! that are meant to be exact, and the arithmetic here - multiplying a rate
//! by a link count - is what the whole report is built out of.

use std::fmt;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Speed {
    mbps: u64,
}

/// The rates Ethernet actually standardised. A rate outside this list still
/// works - the arithmetic does not care - but the report says so, because a
/// number that is not a rate you can buy an optic for is usually a typo.
const STANDARD: [u64; 14] = [
    10, 100, 1_000, 2_500, 5_000, 10_000, 25_000, 40_000, 50_000, 100_000, 200_000, 400_000,
    800_000, 1_600_000,
];

impl Speed {
    pub fn from_mbps(mbps: u64) -> Speed {
        Speed { mbps }
    }

    pub fn mbps(&self) -> u64 {
        self.mbps
    }

    pub fn is_standard(&self) -> bool {
        STANDARD.contains(&self.mbps)
    }

    /// `self` divided by `other`, when it divides exactly. This is the lane
    /// count of a breakout: 400G over 100G is four lanes.
    pub fn lanes_over(&self, other: Speed) -> Option<u32> {
        if other.mbps == 0 || !self.mbps.is_multiple_of(other.mbps) {
            return None;
        }
        u32::try_from(self.mbps / other.mbps).ok()
    }

    /// `self` multiplied by a lane count, for going the other way.
    pub fn times(&self, n: u64) -> Option<Speed> {
        self.mbps.checked_mul(n).map(Speed::from_mbps)
    }

    /// An aggregate rate, meaning this speed carried over `n` links.
    pub fn total(&self, n: u64) -> Total {
        Total(self.mbps.saturating_mul(n))
    }
}

/// A rate that is a sum rather than a port speed, so it is never checked
/// against the standard list - 2.8T of fabric is a perfectly good number and
/// nobody sells an optic for it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Total(u64);

impl fmt::Display for Total {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&render(self.0))
    }
}

impl fmt::Display for Speed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&render(self.mbps))
    }
}

/// Write a rate the way an engineer says it out loud: the largest unit that
/// leaves a number below 1,000, with at most three decimal places and no
/// trailing zeroes. 100,000 is 100G, 1,600,000 is 1.6T, 2,500 is 2.5G.
///
/// Nothing has a port past a terabit, but a total does: a mesh of a few
/// thousand switches adds up to petabits, and writing that as six digits of
/// terabits is a number nobody can read at a glance.
fn render(mbps: u64) -> String {
    let (unit, divisor) = [
        ("E", 1_000_000_000_000),
        ("P", 1_000_000_000),
        ("T", 1_000_000),
        ("G", 1_000),
    ]
    .into_iter()
    .find(|(_, divisor)| mbps >= *divisor)
    .unwrap_or(("M", 1));
    let whole = mbps / divisor;
    let frac = mbps % divisor;
    if frac == 0 {
        return format!("{whole}{unit}");
    }
    // Three decimal places is enough for every rate that exists, and trailing
    // zeroes are noise: 1.600T reads as false precision.
    let scaled = frac * 1_000 / divisor;
    let decimals = format!("{scaled:03}");
    format!("{whole}.{}{unit}", decimals.trim_end_matches('0'))
}

/// Parse a rate as it is written on a datasheet: `100G`, `25g`, `1.6T`,
/// `10M`. A bare number is gigabits, which is what a switch port is quoted in
/// nearly everywhere.
pub fn parse(s: &str) -> Result<Speed, String> {
    let body = s.trim();
    if body.is_empty() {
        return Err("missing a speed: write it like 100G, 25G or 1.6T".into());
    }
    // `100Gb`, `100Gbps` and `100GbE` all mean the same thing and all get
    // typed; taking the suffix off is cheaper than arguing about it.
    let body = strip_suffix_ignoring_case(body, &["bps", "be", "b/s", "b"]);
    let (digits, unit) = match body.chars().last() {
        Some(c) if c.is_ascii_alphabetic() => (&body[..body.len() - 1], c.to_ascii_uppercase()),
        _ => (body, 'G'),
    };
    let multiplier = match unit {
        'M' => 1,
        'G' => 1_000,
        'T' => 1_000_000,
        _ => {
            return Err(format!(
                "'{s}' is not a speed: the unit must be M, G or T, as in 100G"
            ));
        }
    };
    let mbps = scaled(digits, multiplier)
        .ok_or_else(|| format!("'{s}' is not a speed: write it like 100G, 25G or 1.6T"))?;
    if mbps == 0 {
        return Err(format!("'{s}' is a speed of zero"));
    }
    Ok(Speed::from_mbps(mbps))
}

fn strip_suffix_ignoring_case<'a>(s: &'a str, suffixes: &[&str]) -> &'a str {
    for suffix in suffixes {
        if s.len() > suffix.len() && s[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix) {
            return &s[..s.len() - suffix.len()];
        }
    }
    s
}

/// `2.5` times 1,000 as an exact integer, without going through a float.
///
/// The decimal part is padded or truncated against the multiplier, so a rate
/// finer than the multiplier can express - 1.0005T - is rejected rather than
/// silently rounded.
fn scaled(digits: &str, multiplier: u64) -> Option<u64> {
    let (whole, frac) = match digits.split_once('.') {
        Some((w, f)) => (w, f),
        None => (digits, ""),
    };
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut value = whole.parse::<u64>().ok()?.checked_mul(multiplier)?;
    if !frac.is_empty() {
        let scale = 10u64.checked_pow(u32::try_from(frac.len()).ok()?)?;
        let numerator = frac.parse::<u64>().ok()?.checked_mul(multiplier)?;
        if !numerator.is_multiple_of(scale) {
            return None;
        }
        value = value.checked_add(numerator / scale)?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Speed {
        parse(s).expect("parses")
    }

    #[test]
    fn reads_the_usual_rates() {
        assert_eq!(p("100G").mbps(), 100_000);
        assert_eq!(p("25g").mbps(), 25_000);
        assert_eq!(p("400G").mbps(), 400_000);
        assert_eq!(p("1.6T").mbps(), 1_600_000);
        assert_eq!(p("2.5G").mbps(), 2_500);
        assert_eq!(p("100M").mbps(), 100);
    }

    #[test]
    fn a_bare_number_is_gigabits() {
        assert_eq!(p("100").mbps(), 100_000);
        assert_eq!(p("10").mbps(), 10_000);
    }

    #[test]
    fn tolerates_how_people_write_it() {
        for s in ["100Gb", "100Gbps", "100GbE", "100gb/s", " 100G "] {
            assert_eq!(p(s).mbps(), 100_000, "{s}");
        }
    }

    #[test]
    fn rejects_what_is_not_a_speed() {
        for s in ["", "banana", "100K", "G", "1.2.3G", "-100G", "0G", "100 G"] {
            assert!(parse(s).is_err(), "{s} should not parse");
        }
    }

    #[test]
    fn rejects_a_rate_finer_than_a_megabit() {
        // A megabit is the unit, so anything finer is refused rather than
        // quietly rounded. 1.0005T is 1,000,500 Mb/s and survives; half a
        // megabit does not.
        assert_eq!(p("1.0005T").mbps(), 1_000_500);
        assert!(parse("1.0000005T").is_err());
        assert!(parse("1.5M").is_err());
    }

    #[test]
    fn writes_rates_the_way_they_are_said() {
        let cases = [
            (100_000, "100G"),
            (1_600_000, "1.6T"),
            (2_800_000, "2.8T"),
            (700_000, "700G"),
            (2_500, "2.5G"),
            (100, "100M"),
            (1_000_000, "1T"),
            (1_000, "1G"),
            // A big enough fabric adds up past a terabit.
            (419_430_400_000, "419.43P"),
            (838_656_000_000, "838.656P"),
            (2_000_000_000_000, "2E"),
        ];
        for (mbps, want) in cases {
            assert_eq!(Speed::from_mbps(mbps).to_string(), want);
        }
    }

    #[test]
    fn a_round_trip_survives_writing_and_reading() {
        for mbps in [100, 2_500, 25_000, 100_000, 400_000, 1_600_000] {
            let s = Speed::from_mbps(mbps);
            assert_eq!(parse(&s.to_string()).unwrap(), s);
        }
    }

    #[test]
    fn lanes_divide_exactly_or_not_at_all() {
        assert_eq!(p("400G").lanes_over(p("100G")), Some(4));
        assert_eq!(p("800G").lanes_over(p("100G")), Some(8));
        assert_eq!(p("40G").lanes_over(p("10G")), Some(4));
        assert_eq!(p("100G").lanes_over(p("40G")), None);
        assert_eq!(p("25G").lanes_over(p("100G")), None);
    }

    #[test]
    fn knows_which_rates_are_standard() {
        assert!(p("100G").is_standard());
        assert!(p("1.6T").is_standard());
        assert!(!p("300G").is_standard());
        assert!(!p("7G").is_standard());
    }

    #[test]
    fn totals_are_not_port_speeds() {
        assert_eq!(p("100G").total(28).to_string(), "2.8T");
        assert_eq!(p("100G").total(7).to_string(), "700G");
        assert_eq!(p("100G").total(0).to_string(), "0M");
    }
}
