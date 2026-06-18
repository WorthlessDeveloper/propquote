//! Fit Obric-form parameters (`big_k`, `fee_millionth`) to ground-truth samples.
//!
//! The two parameters are nearly degenerate at small trade sizes (both scale the output down), so
//! we decouple them: `big_k` controls curve *shape* and `fee` controls *level*. For any candidate
//! `big_k` we find the best fee by a 1-D search, leaving a clean 1-D problem in `big_k` that we
//! solve with a coarse power-of-two bracket followed by a step-halving hill-climb.

use propquote_core::amm::PropAmm;
use propquote_obric::{ObricPool, SSTradingPair};

use crate::sample::Sample;

const FEE_MAX: u64 = 1_000_000;
/// Penalty added per sample the candidate params cannot quote (overflow / insufficient liquidity).
const MISS_PENALTY: u128 = 1u128 << 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObricParams {
    pub big_k: u128,
    pub fee_millionth: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct FitResult {
    pub params: ObricParams,
    pub samples: usize,
    /// Worst absolute output error across the samples, in raw token units.
    pub max_abs_error: u64,
    /// Worst relative output error across the samples, in basis points. This is the metric to
    /// judge a fit by — sub-bp means the closed form predicts the venue essentially exactly.
    pub max_error_bps: f64,
}

/// Predict a single sample's output under candidate `params`.
pub fn predict(sample: &Sample, params: ObricParams) -> Option<u64> {
    let state = SSTradingPair {
        is_initialized: true,
        mult_x: sample.mult_x,
        mult_y: sample.mult_y,
        big_k: params.big_k,
        target_x: sample.target_x,
        fee_millionth: params.fee_millionth,
        ..Default::default()
    };
    let mut pool = ObricPool::new(state);
    pool.set_reserves(sample.current_x, sample.current_y);
    pool.quote_in(sample.side, sample.amount_in)
        .ok()
        .map(|q| q.amount_out)
}

fn abs_diff(a: u64, b: u64) -> u64 {
    a.abs_diff(b)
}

fn total_err(samples: &[Sample], params: ObricParams) -> u128 {
    let mut acc: u128 = 0;
    for s in samples {
        match predict(s, params) {
            Some(o) => acc = acc.saturating_add(abs_diff(o, s.amount_out) as u128),
            None => acc = acc.saturating_add(MISS_PENALTY),
        }
    }
    acc
}

/// Best `fee_millionth` for a fixed `big_k` (output is monotone in fee, so the error is unimodal).
fn best_fee(samples: &[Sample], big_k: u128) -> u64 {
    let (mut lo, mut hi) = (0u64, FEE_MAX);
    while hi - lo > 2 {
        let third = (hi - lo) / 3;
        let (m1, m2) = (lo + third, hi - third);
        let e1 = total_err(
            samples,
            ObricParams {
                big_k,
                fee_millionth: m1,
            },
        );
        let e2 = total_err(
            samples,
            ObricParams {
                big_k,
                fee_millionth: m2,
            },
        );
        if e1 <= e2 {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let mut best = lo;
    let mut best_e = total_err(
        samples,
        ObricParams {
            big_k,
            fee_millionth: lo,
        },
    );
    for f in (lo + 1)..=hi {
        let e = total_err(
            samples,
            ObricParams {
                big_k,
                fee_millionth: f,
            },
        );
        if e < best_e {
            best_e = e;
            best = f;
        }
    }
    best
}

/// Error at `big_k` using its best-fit fee — the 1-D objective we minimize.
fn err_at(samples: &[Sample], big_k: u128) -> u128 {
    total_err(
        samples,
        ObricParams {
            big_k,
            fee_millionth: best_fee(samples, big_k),
        },
    )
}

/// Fit the Obric form to `samples`. Returns `None` only if `samples` is empty or no candidate
/// could quote every sample.
pub fn fit_obric_form(samples: &[Sample]) -> Option<FitResult> {
    if samples.is_empty() {
        return None;
    }

    // 1. Coarse bracket: pick the power-of-two octave of big_k with the lowest error.
    let mut big_k: u128 = 2;
    let mut best_e = err_at(samples, big_k);
    for i in 2..=120u32 {
        let cand = 1u128 << i;
        let e = err_at(samples, cand);
        if e < best_e {
            best_e = e;
            big_k = cand;
        }
    }

    // 2. Step-halving hill-climb to the exact integer minimum.
    let mut cur = err_at(samples, big_k);
    let mut step = (big_k / 2).max(1);
    loop {
        loop {
            let mut moved = false;
            let neighbours = [big_k.checked_sub(step), big_k.checked_add(step)];
            for c in neighbours.into_iter().flatten() {
                if c > 0 {
                    let e = err_at(samples, c);
                    if e < cur {
                        big_k = c;
                        cur = e;
                        moved = true;
                    }
                }
            }
            if !moved {
                break;
            }
        }
        if step == 1 {
            break;
        }
        step = (step / 2).max(1);
    }

    let params = ObricParams {
        big_k,
        fee_millionth: best_fee(samples, big_k),
    };

    // 3. Residuals.
    let mut max_abs = 0u64;
    let mut max_bps = 0.0f64;
    for s in samples {
        let o = predict(s, params)?;
        let d = abs_diff(o, s.amount_out);
        if d > max_abs {
            max_abs = d;
        }
        if s.amount_out > 0 {
            let bps = d as f64 / s.amount_out as f64 * 10_000.0;
            if bps > max_bps {
                max_bps = bps;
            }
        }
    }

    Some(FitResult {
        params,
        samples: samples.len(),
        max_abs_error: max_abs,
        max_error_bps: max_bps,
    })
}
