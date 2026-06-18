//! Overflow-safe fixed-width integer math.
//!
//! On-chain AMM math is done in `u128`/`u256`-ish space and is sensitive to rounding. Magnus's
//! Obric port writes things like `big_k * mult_y / mult_x` directly, which panics if the
//! multiplication overflows `u128`. We instead compute `floor(a*b/denom)` at full 256-bit
//! precision so the same expression is always correct, and provide a non-panicking integer sqrt.

/// `floor(a * b / denom)` computed via a 256-bit intermediate product.
///
/// Returns `None` if `denom == 0` or if the exact quotient does not fit in `u128`.
/// For the common case where `a * b` fits in `u128`, this is just a multiply and divide.
pub fn mul_div_floor(a: u128, b: u128, denom: u128) -> Option<u128> {
    if denom == 0 {
        return None;
    }
    if let Some(prod) = a.checked_mul(b) {
        return Some(prod / denom);
    }
    let (hi, lo) = mul_full(a, b);
    div_256_by_128(hi, lo, denom)
}

const MASK64: u128 = 0xFFFF_FFFF_FFFF_FFFF;

/// Full 128x128 -> 256-bit product, returned as `(hi, lo)`.
fn mul_full(a: u128, b: u128) -> (u128, u128) {
    let (a0, a1) = (a & MASK64, a >> 64);
    let (b0, b1) = (b & MASK64, b >> 64);

    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;

    let r0 = p00 & MASK64;
    let mut carry = (p00 >> 64) + (p01 & MASK64) + (p10 & MASK64);
    let r1 = carry & MASK64;
    carry = (carry >> 64) + (p01 >> 64) + (p10 >> 64) + (p11 & MASK64);
    let r2 = carry & MASK64;
    let r3 = (carry >> 64) + (p11 >> 64);

    let lo = r0 | (r1 << 64);
    let hi = r2 | (r3 << 64);
    (hi, lo)
}

/// Divide a 256-bit value `(hi, lo)` by a `u128` divisor, returning the quotient if it fits in
/// `u128`. Binary long division; the `carry_out` trick keeps the running remainder within `u128`.
fn div_256_by_128(hi: u128, lo: u128, d: u128) -> Option<u128> {
    if hi == 0 {
        return Some(lo / d);
    }
    // If hi >= d the quotient is >= 2^128 and cannot fit in u128.
    if hi >= d {
        return None;
    }

    let mut quotient: u128 = 0;
    let mut rem: u128 = 0;
    let mut i: i32 = 255;
    while i >= 0 {
        let bit = if i >= 128 {
            (hi >> (i - 128)) & 1
        } else {
            (lo >> i) & 1
        };
        let carry_out = rem >> 127;
        rem = (rem << 1) | bit;
        if carry_out == 1 || rem >= d {
            rem = rem.wrapping_sub(d);
            // With hi < d the quotient is < 2^128, so no bit at index >= 128 is ever set.
            if i < 128 {
                quotient |= 1u128 << i;
            }
        }
        i -= 1;
    }
    Some(quotient)
}

/// Floor of the integer square root of `n` (Newton's method, overflow-free).
pub fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    // `x.div_ceil(2)` rather than `(x + 1) / 2` so the initial guess can't overflow at u128::MAX.
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_div_fast_path() {
        assert_eq!(mul_div_floor(10, 20, 4), Some(50));
        assert_eq!(mul_div_floor(7, 3, 2), Some(10)); // floor(21/2)
        assert_eq!(mul_div_floor(0, 12345, 7), Some(0));
        assert_eq!(mul_div_floor(1, 1, 0), None);
    }

    #[test]
    fn mul_div_wide_path() {
        // a*b overflows u128, result still fits.
        assert_eq!(mul_div_floor(u128::MAX, 1, 2), Some(u128::MAX / 2));
        assert_eq!(mul_div_floor(u128::MAX, 4, 4), Some(u128::MAX));
        assert_eq!(mul_div_floor(1 << 100, 1 << 30, 1 << 20), Some(1 << 110));
        // Result would exceed u128 -> None.
        assert_eq!(mul_div_floor(u128::MAX, 4, 2), None);
        assert_eq!(mul_div_floor(u128::MAX, u128::MAX, 1), None);
    }

    #[test]
    fn mul_div_matches_reference_when_no_overflow() {
        // Cross-check the wide path against a u64-domain reference.
        let cases = [
            (123456789u128, 987654321u128, 1000u128),
            (u64::MAX as u128, u64::MAX as u128, 7),
            ((1u128 << 90) + 5, (1u128 << 30) + 9, (1u128 << 20) + 1),
        ];
        for (a, b, d) in cases {
            let (hi, lo) = mul_full(a, b);
            // Reconstruct via the wide divider and compare to the fast path when it is valid.
            let wide = div_256_by_128(hi, lo, d);
            if let Some(prod) = a.checked_mul(b) {
                assert_eq!(wide, Some(prod / d), "a={a} b={b} d={d}");
            }
        }
    }

    #[test]
    fn isqrt_known() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(99), 9);
        assert_eq!(isqrt(100), 10);
        assert_eq!(isqrt(101), 10);
        assert_eq!(isqrt(100_000_000_000_000), 10_000_000);
    }

    #[test]
    fn isqrt_floor_property() {
        for n in [2u128, 3, 15, 16, 17, 1_000_003, u128::MAX] {
            let r = isqrt(n);
            // r*r never overflows: r <= 2^64-1 for n <= u128::MAX, so r*r < 2^128.
            assert!(r * r <= n, "r*r <= n for n={n}");
            // (r+1)^2 > n — but (r+1)^2 can exceed u128, which trivially satisfies it.
            match (r + 1).checked_mul(r + 1) {
                Some(sq) => assert!(sq > n, "(r+1)^2 > n for n={n}"),
                None => {} // (r+1)^2 > u128::MAX >= n
            }
        }
    }
}
