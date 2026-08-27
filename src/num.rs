//! Big-integer-free helpers for talking about address counts.
//!
//! A prefix of length `L` in a family with `M`-bit addresses holds `2^(M-L)`
//! addresses, which for IPv6 overflows every native integer type. Rather than
//! pull in a bignum crate we keep the exponent and render decimal digits by
//! repeated doubling, which is plenty fast for exponents up to 128.

/// The number of addresses in a prefix, held as a power of two.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Count {
    exp: u32,
}

impl Count {
    pub fn pow2(exp: u32) -> Count {
        Count { exp }
    }

    /// Exact value when it fits in a `u128`, otherwise `None` (only 2^128).
    pub fn as_u128(&self) -> Option<u128> {
        if self.exp < 128 {
            Some(1u128 << self.exp)
        } else {
            None
        }
    }

    /// Exact decimal digits, no grouping.
    pub fn digits(&self) -> String {
        pow2_digits(self.exp)
    }

    /// Exact decimal value with thousands separators.
    pub fn grouped(&self) -> String {
        group(&self.digits())
    }

    /// The count as a plain figure: exact while that is readable, and a power
    /// of two once it is not.
    pub fn short(&self) -> String {
        if self.exp <= READABLE_BITS {
            self.grouped()
        } else {
            format!("2^{}", self.exp)
        }
    }

    /// The count with room to spare, for the headline address total.
    pub fn describe(&self) -> String {
        if self.exp <= READABLE_BITS {
            self.grouped()
        } else {
            format!("2^{} (~{})", self.exp, self.approx())
        }
    }

    /// Short scientific-ish approximation, e.g. `3.4e38`.
    pub fn approx(&self) -> String {
        scientific(f64::from(self.exp) * std::f64::consts::LOG10_2)
    }
}

/// How many bits of magnitude are worth spelling out in full.
///
/// A /96 holds 79,228,162,514,264,337,593,543,950,336 addresses. Nobody reads
/// that; they read `2^96`. Past this width the exact digits are noise that
/// pushes the useful part of the line off the screen, so they are dropped in
/// favour of the power of two and an order of magnitude. Machine-readable
/// output is unaffected - `--json` still carries exact integers.
const READABLE_BITS: u32 = 32;

/// Render a base-ten logarithm as `1.8e19`.
fn scientific(log10: f64) -> String {
    let whole = log10.floor();
    let mantissa = 10f64.powf(log10 - whole);
    format!("{:.1}e{}", mantissa, whole as i64)
}

/// Total of several counts, exact while that is readable and an order of
/// magnitude beyond it.
pub fn describe_sum(counts: &[Count]) -> String {
    // A single block is just itself, and says so even at 2^128, where the
    // total no longer fits in a u128 to be examined.
    if let [only] = counts {
        return only.describe();
    }
    match exact_sum(counts) {
        Some(total) if total <= 1u128 << READABLE_BITS => group(&total.to_string()),
        // A total that happens to be a power of two is worth saying so.
        Some(total) if total.is_power_of_two() => {
            let exp = total.trailing_zeros();
            format!("2^{exp} (~{})", Count::pow2(exp).approx())
        }
        _ => format!(
            "~{}",
            scientific(log2_sum(counts) * std::f64::consts::LOG10_2)
        ),
    }
}

/// `None` when the total will not fit in a `u128`, which only the very largest
/// IPv6 spans manage.
fn exact_sum(counts: &[Count]) -> Option<u128> {
    counts
        .iter()
        .try_fold(0u128, |acc, c| acc.checked_add(c.as_u128()?))
}

/// Base-two logarithm of the total, computed relative to the largest term so
/// the remaining terms stay well inside what an f64 can hold.
fn log2_sum(counts: &[Count]) -> f64 {
    let Some(max) = counts.iter().map(|c| c.exp).max() else {
        return 0.0;
    };
    let multiplier: f64 = counts
        .iter()
        .map(|c| 2f64.powi(-i32::try_from(max - c.exp).unwrap_or(i32::MAX)))
        .sum();
    f64::from(max) + multiplier.log2()
}

/// Sum of several powers of two, rendered exactly. Machine-readable output
/// keeps the full digits; the human report uses [`describe_sum`].
pub fn sum_grouped(counts: &[Count]) -> String {
    let mut total = vec![0u8]; // little-endian decimal digits
    for c in counts {
        let digits: Vec<u8> = pow2_digits(c.exp).bytes().rev().map(|b| b - b'0').collect();
        add_into(&mut total, &digits);
    }
    let s: String = total.iter().rev().map(|d| (b'0' + d) as char).collect();
    group(&s)
}

fn pow2_digits(exp: u32) -> String {
    let mut digits = vec![1u8]; // little-endian
    for _ in 0..exp {
        let mut carry = 0u8;
        for d in digits.iter_mut() {
            let v = *d * 2 + carry;
            *d = v % 10;
            carry = v / 10;
        }
        if carry > 0 {
            digits.push(carry);
        }
    }
    digits.iter().rev().map(|d| (b'0' + d) as char).collect()
}

fn add_into(acc: &mut Vec<u8>, other: &[u8]) {
    let mut carry = 0u8;
    for i in 0..other.len().max(acc.len()) {
        if i == acc.len() {
            acc.push(0);
        }
        let v = acc[i] + other.get(i).copied().unwrap_or(0) + carry;
        acc[i] = v % 10;
        carry = v / 10;
    }
    if carry > 0 {
        acc.push(carry);
    }
}

/// Insert `,` every three digits from the right.
pub fn group(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let n = s.len();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (n - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_powers_are_exact() {
        assert_eq!(Count::pow2(0).digits(), "1");
        assert_eq!(Count::pow2(8).digits(), "256");
        assert_eq!(Count::pow2(64).digits(), "18446744073709551616");
    }

    #[test]
    fn full_ipv6_space_does_not_overflow() {
        assert_eq!(
            Count::pow2(128).digits(),
            "340282366920938463463374607431768211456"
        );
        assert_eq!(Count::pow2(128).as_u128(), None);
    }

    #[test]
    fn grouping() {
        assert_eq!(group("1"), "1");
        assert_eq!(group("1000"), "1,000");
        assert_eq!(group("1234567"), "1,234,567");
    }

    #[test]
    fn exact_digits_stop_at_the_readable_width() {
        // 2^32 is the last size worth spelling out.
        assert_eq!(Count::pow2(32).describe(), "4,294,967,296");
        assert_eq!(Count::pow2(32).short(), "4,294,967,296");
        // Past it, the power of two carries the meaning.
        assert_eq!(Count::pow2(33).short(), "2^33");
        assert_eq!(Count::pow2(33).describe(), "2^33 (~8.6e9)");
        assert_eq!(Count::pow2(76).describe(), "2^76 (~7.6e22)");
        assert_eq!(Count::pow2(128).describe(), "2^128 (~3.4e38)");
        // Small counts are untouched.
        assert_eq!(Count::pow2(8).describe(), "256");
        assert_eq!(Count::pow2(0).describe(), "1");
    }

    #[test]
    fn no_readable_number_runs_away_with_the_line() {
        // Nothing the human report prints should be a wall of digits.
        for exp in 0..=128 {
            for rendered in [Count::pow2(exp).short(), Count::pow2(exp).describe()] {
                let digits = rendered.chars().filter(char::is_ascii_digit).count();
                assert!(
                    digits <= 13,
                    "2^{exp} rendered as {rendered:?}, {digits} digits"
                );
            }
        }
    }

    #[test]
    fn totals_are_exact_while_they_are_readable() {
        assert_eq!(describe_sum(&[]), "0");
        assert_eq!(describe_sum(&[Count::pow2(8), Count::pow2(8)]), "512");
        // 2^31 + 2^31 == 2^32, still readable.
        assert_eq!(
            describe_sum(&[Count::pow2(31), Count::pow2(31)]),
            "4,294,967,296"
        );
    }

    #[test]
    fn large_totals_are_summarised() {
        // A power of two says so.
        assert_eq!(
            describe_sum(&[Count::pow2(32), Count::pow2(32)]),
            "2^33 (~8.6e9)"
        );
        // A ragged total gets an order of magnitude. 2^75 + 2^74 is 5.7e22.
        assert_eq!(describe_sum(&[Count::pow2(75), Count::pow2(74)]), "~5.7e22");
        // A single block describes itself even past what a u128 can total.
        assert_eq!(describe_sum(&[Count::pow2(128)]), "2^128 (~3.4e38)");
    }

    #[test]
    fn the_approximation_tracks_the_exact_total() {
        // Where both are available they must agree to within rounding.
        for counts in [
            vec![Count::pow2(40), Count::pow2(39)],
            vec![Count::pow2(64), Count::pow2(1)],
            vec![Count::pow2(50); 5],
        ] {
            let exact = exact_sum(&counts).expect("fits") as f64;
            let approx = 2f64.powf(log2_sum(&counts));
            assert!(
                (approx - exact).abs() / exact < 1e-9,
                "{approx} is not close to {exact}"
            );
        }
    }

    #[test]
    fn machine_output_keeps_every_digit() {
        // describe_sum is for people; sum_grouped and digits() stay exact.
        assert_eq!(Count::pow2(76).digits(), "75557863725914323419136");
        assert_eq!(
            sum_grouped(&[Count::pow2(75), Count::pow2(74)]),
            "56,668,397,794,435,742,564,352"
        );
    }

    #[test]
    fn sums_mixed_sizes() {
        // 2^8 + 2^8 == 2^9
        let s = sum_grouped(&[Count::pow2(8), Count::pow2(8)]);
        assert_eq!(s, "512");
        // a /53 plus a /54 worth of a v6 space
        let s = sum_grouped(&[Count::pow2(75), Count::pow2(74)]);
        assert_eq!(s, "56,668,397,794,435,742,564,352");
    }
}
