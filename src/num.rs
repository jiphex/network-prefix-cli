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

    /// `12,345 (2^14)` - the exact count plus the power of two, which is the
    /// form network engineers actually reason in once the numbers get silly.
    pub fn describe(&self) -> String {
        if self.exp <= 10 {
            self.grouped()
        } else if self.exp <= 40 {
            format!("{} (2^{})", self.grouped(), self.exp)
        } else {
            format!("{} (2^{}, ~{})", self.grouped(), self.exp, self.approx())
        }
    }

    /// Short scientific-ish approximation, e.g. `3.4e38`.
    pub fn approx(&self) -> String {
        let log10 = f64::from(self.exp) * std::f64::consts::LOG10_2;
        let whole = log10.floor();
        let mantissa = 10f64.powf(log10 - whole);
        format!("{:.1}e{}", mantissa, whole as i64)
    }
}

/// Sum of several powers of two, rendered exactly. Used for "space remaining"
/// where the free blocks are of assorted sizes.
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
    fn sums_mixed_sizes() {
        // 2^8 + 2^8 == 2^9
        let s = sum_grouped(&[Count::pow2(8), Count::pow2(8)]);
        assert_eq!(s, "512");
        // a /53 plus a /54 worth of a v6 space
        let s = sum_grouped(&[Count::pow2(75), Count::pow2(74)]);
        assert_eq!(s, "56,668,397,794,435,742,564,352");
    }
}
