//! Closed-form Obric V2 quoting: exact-in, exact-out, and arb-grade price metrics.

use propquote_core::amm::PropAmm;
use propquote_core::bs58;
use propquote_core::math::{isqrt, mul_div_floor};
use propquote_core::types::{Pubkey, QuoteError, QuoteResult, Side, SwapMode};

use crate::state::SSTradingPair;

/// Obric V2 program id (base58).
pub const OBRIC_V2_PROGRAM_ID: &str = "obriQD1zbpyLz95G5n7nJe6a4DPjpFwa5XYPoNm113y";

const MILLION: u128 = 1_000_000;

/// A live Obric pool: decoded config + current vault balances. Quotes are pure functions of this.
#[derive(Clone, Debug)]
pub struct ObricPool {
    pub state: SSTradingPair,
    /// Current X reserve (raw token units, from the `reserve_x` vault).
    pub current_x: u128,
    /// Current Y reserve (raw token units, from the `reserve_y` vault).
    pub current_y: u128,
}

/// Internal: the per-side geometry needed by both exact-in and the metrics.
struct Curve {
    big_k: u128,
    current_x_k: u128,
    target: u128,
    /// Current reserve of the *input* token (for the deficit/rebate term).
    current_in: u128,
    /// Current reserve of the *output* token (the liquidity bound).
    current_out: u128,
    /// Oracle price as output-per-input.
    oracle_price: f64,
}

impl ObricPool {
    pub fn new(state: SSTradingPair) -> Self {
        ObricPool { state, current_x: 0, current_y: 0 }
    }

    /// Set the current vault balances (raw token units).
    pub fn set_reserves(&mut self, current_x: u64, current_y: u64) -> &mut Self {
        self.current_x = current_x as u128;
        self.current_y = current_y as u128;
        self
    }

    /// `target_x_k = sqrt(big_k * mult_y / mult_x)` — the curve-K point where marginal price equals
    /// the oracle price. Overflow-safe (the multiply is done at 256-bit width).
    fn target_x_k(&self) -> Result<u128, QuoteError> {
        let v = mul_div_floor(self.state.big_k, self.state.mult_y as u128, self.state.mult_x as u128)
            .ok_or(QuoteError::MathOverflow)?;
        Ok(isqrt(v))
    }

    /// `(target_x, target_y)` in token units, value-rebalanced via the oracle mults.
    fn target_xy(&self) -> Result<(u128, u128), QuoteError> {
        let mx = self.state.mult_x as u128;
        let my = self.state.mult_y as u128;
        if mx == 0 || my == 0 {
            return Err(QuoteError::InvalidData);
        }
        let value_x = self.current_x.checked_mul(mx).ok_or(QuoteError::MathOverflow)?;
        let value_y = self.current_y.checked_mul(my).ok_or(QuoteError::MathOverflow)?;
        let value_total = value_x.checked_add(value_y).ok_or(QuoteError::MathOverflow)?;
        let target_x = self.state.target_x as u128;
        let target_x_value = target_x.checked_mul(mx).ok_or(QuoteError::MathOverflow)?;
        let target_y_value = value_total.checked_sub(target_x_value).ok_or(QuoteError::Underflow)?;
        let target_y = target_y_value / my;
        Ok((target_x, target_y))
    }

    /// Build the per-side curve geometry. `current_x_k` shifts real reserves onto curve-K,
    /// anchored at the inventory target.
    fn curve(&self, side: Side) -> Result<Curve, QuoteError> {
        let big_k = self.state.big_k;
        let target_x_k = self.target_x_k()?;
        let (target_x, target_y) = self.target_xy()?;

        // current_x_k = target_x_k - target_x + current_x  (guarded against underflow)
        let current_x_k = target_x_k
            .checked_add(self.current_x)
            .ok_or(QuoteError::MathOverflow)?
            .checked_sub(target_x)
            .ok_or(QuoteError::Underflow)?;
        if current_x_k == 0 {
            return Err(QuoteError::InsufficientLiquidity);
        }

        let mx = self.state.mult_x as f64;
        let my = self.state.mult_y as f64;
        let (target, current_in, current_out, oracle_price) = match side {
            // Selling X for Y: oracle price is Y-per-X = mult_x / mult_y.
            Side::XtoY => (target_x, self.current_x, self.current_y, mx / my),
            // Selling Y for X: oracle price is X-per-Y = mult_y / mult_x.
            Side::YtoX => (target_y, self.current_y, self.current_x, my / mx),
        };

        Ok(Curve { big_k, current_x_k, target, current_in, current_out, oracle_price })
    }

    /// Exact-in quote for either side.
    fn quote_exact_in(&self, side: Side, amount_in: u64) -> Result<QuoteResult, QuoteError> {
        if amount_in == 0 {
            return Err(QuoteError::ZeroAmount);
        }
        let c = self.curve(side)?;
        let input = amount_in as u128;

        // Walk the constant-product invariant on curve-K. The input side always *adds* to its
        // curve-K coordinate; we then read the other coordinate off `big_k / coord`.
        let (current_in_k, out_before_fee, marginal_before, marginal_after) = match side {
            Side::XtoY => {
                let current_x_k = c.current_x_k;
                let current_y_k = c.big_k / current_x_k;
                let new_x_k = current_x_k.checked_add(input).ok_or(QuoteError::MathOverflow)?;
                let new_y_k = c.big_k / new_x_k;
                let out = current_y_k.checked_sub(new_y_k).ok_or(QuoteError::Underflow)?;
                let mb = marginal(c.big_k, current_x_k);
                let ma = marginal(c.big_k, new_x_k);
                (current_x_k, out, mb, ma)
            }
            Side::YtoX => {
                let current_x_k = c.current_x_k;
                let current_y_k = c.big_k / current_x_k;
                let new_y_k = current_y_k.checked_add(input).ok_or(QuoteError::MathOverflow)?;
                let new_x_k = c.big_k / new_y_k;
                let out = current_x_k.checked_sub(new_x_k).ok_or(QuoteError::Underflow)?;
                let mb = marginal(c.big_k, current_y_k);
                let ma = marginal(c.big_k, new_y_k);
                (current_y_k, out, mb, ma)
            }
        };
        let _ = current_in_k;

        // The pool cannot pay out more than it holds of the output token.
        if out_before_fee >= c.current_out {
            return Err(QuoteError::InsufficientLiquidity);
        }

        // Fee with inventory rebate. Evaluated left-to-right to match the on-chain truncation.
        let fee_before_rebate =
            mul_div_floor(out_before_fee, self.state.fee_millionth as u128, MILLION)
                .ok_or(QuoteError::MathOverflow)?;
        let deficit = c.target.saturating_sub(core::cmp::min(c.target, c.current_in));
        let rebate_ratio = core::cmp::min(input, deficit) * 100 / input; // input > 0
        let mut rebate = fee_before_rebate
            .checked_mul(rebate_ratio)
            .ok_or(QuoteError::MathOverflow)?;
        rebate /= 100;
        rebate = rebate
            .checked_mul(self.state.rebate_percentage as u128)
            .ok_or(QuoteError::MathOverflow)?;
        rebate /= 100;
        let fee = fee_before_rebate - rebate;
        let out_after_fee = out_before_fee - fee;
        // Note: `out_after_fee == 0` is a valid "dust" quote (input too small to yield a unit),
        // not an error — exact-out relies on that to grow its search. The genuine liquidity wall
        // is the `out_before_fee >= current_out` check above.
        let protocol_fee = fee
            .checked_mul(self.state.protocol_fee_share_thousandth as u128)
            .ok_or(QuoteError::MathOverflow)?
            / 1000;
        let lp_fee = fee - protocol_fee;

        let effective_price = out_after_fee as f64 / input as f64;
        let price_impact_bps = if c.oracle_price > 0.0 {
            (c.oracle_price - effective_price) / c.oracle_price * 10_000.0
        } else {
            0.0
        };

        Ok(QuoteResult {
            amount_in,
            amount_out: out_after_fee as u64,
            fee: fee as u64,
            protocol_fee: protocol_fee as u64,
            lp_fee: lp_fee as u64,
            oracle_price: c.oracle_price,
            effective_price,
            marginal_price_before: marginal_before,
            marginal_price_after: marginal_after,
            price_impact_bps,
        })
    }

    /// Exact-out via monotonic binary search over the exact-in function. Robust to the fee/rebate
    /// nonlinearity (which closed-form inversion would have to approximate).
    fn quote_exact_out(&self, side: Side, desired_out: u64) -> Result<QuoteResult, QuoteError> {
        if desired_out == 0 {
            return Err(QuoteError::ZeroAmount);
        }
        // Can't ever pay out more than the output reserve holds.
        let output_reserve = match side {
            Side::XtoY => self.current_y,
            Side::YtoX => self.current_x,
        };
        if desired_out as u128 >= output_reserve {
            return Err(QuoteError::InsufficientLiquidity);
        }
        let out_of = |amt: u64| self.quote_exact_in(side, amt).map(|q| q.amount_out);

        // Exponentially grow an upper bound until it fills the requested size.
        let mut hi: u64 = 1;
        loop {
            match out_of(hi) {
                Ok(o) if o >= desired_out => break,
                Ok(_) => {} // includes dust quotes (out == 0); keep growing
                // Growing the input hit the reserve wall before reaching the target: unfillable.
                Err(QuoteError::InsufficientLiquidity) => {
                    return Err(QuoteError::InsufficientLiquidity)
                }
                Err(e) => return Err(e),
            }
            hi = hi.checked_mul(2).ok_or(QuoteError::NotConverged)?;
        }

        // Binary search the smallest input whose output meets the target.
        let mut lo: u64 = hi / 2; // out(lo) < desired_out by construction
        while lo + 1 < hi {
            let mid = lo + (hi - lo) / 2;
            match out_of(mid) {
                Ok(o) if o >= desired_out => hi = mid,
                _ => lo = mid,
            }
        }
        self.quote_exact_in(side, hi)
    }
}

/// Marginal price (output-per-input) at a curve-K coordinate: `d(out)/d(in) = big_k / coord^2`.
fn marginal(big_k: u128, coord: u128) -> f64 {
    if coord == 0 {
        return 0.0;
    }
    let c = coord as f64;
    big_k as f64 / (c * c)
}

impl PropAmm for ObricPool {
    fn label(&self) -> &'static str {
        "ObricV2"
    }

    fn program_id(&self) -> Pubkey {
        bs58::decode_32(OBRIC_V2_PROGRAM_ID).unwrap_or([0u8; 32])
    }

    fn accounts_to_watch(&self) -> Vec<Pubkey> {
        vec![
            self.state.reserve_x,
            self.state.reserve_y,
            self.state.x_price_feed_id,
            self.state.y_price_feed_id,
        ]
    }

    fn quote(&self, side: Side, amount: u64, mode: SwapMode) -> Result<QuoteResult, QuoteError> {
        match mode {
            SwapMode::ExactIn => self.quote_exact_in(side, amount),
            SwapMode::ExactOut => self.quote_exact_out(side, amount),
        }
    }
}
