//! Fitter tests: generate ground-truth samples from a known Obric pool, then prove the fitter
//! recovers a closed form that predicts them to sub-bp accuracy (the metric that matters for arb).

use propquote_core::amm::PropAmm;
use propquote_core::types::Side;
use propquote_obric::{ObricPool, SSTradingPair};
use propquote_replay::{fit_obric_form, predict, Sample};

const MX: u64 = 1_000;
const MY: u64 = 1_000;
const TARGET: u64 = 5_000_000_000_000; // 5e12, equal to reserves -> no rebate

/// Generate samples by quoting a known Obric pool across a range of sizes.
fn gen_samples(big_k: u128, fee: u64, sizes: &[u64]) -> Vec<Sample> {
    let state = SSTradingPair {
        is_initialized: true,
        mult_x: MX,
        mult_y: MY,
        big_k,
        target_x: TARGET,
        fee_millionth: fee,
        ..Default::default()
    };
    let mut pool = ObricPool::new(state);
    pool.set_reserves(TARGET, TARGET);

    sizes
        .iter()
        .map(|&amt| {
            let out = pool.quote_in(Side::XtoY, amt).unwrap().amount_out;
            Sample {
                current_x: TARGET,
                current_y: TARGET,
                mult_x: MX,
                mult_y: MY,
                target_x: TARGET,
                side: Side::XtoY,
                amount_in: amt,
                amount_out: out,
            }
        })
        .collect()
}

#[test]
fn recovers_concentrated_pool_essentially_exactly() {
    let big_k = 250_000_000_000_000_000_000_000_000_000u128; // 2.5e29
    let fee = 2_000u64;
    let samples = gen_samples(
        big_k,
        fee,
        &[
            1_000_000_000,
            10_000_000_000,
            100_000_000_000,
            500_000_000_000,
            1_000_000_000_000,
            2_000_000_000_000,
            4_000_000_000_000,
        ],
    );

    let r = fit_obric_form(&samples).unwrap();
    assert!(
        r.max_error_bps < 0.01,
        "max_error_bps = {}",
        r.max_error_bps
    );
    // Well-conditioned: the fit is exact to within rounding dust.
    assert!(r.max_abs_error <= 2, "max_abs_error = {}", r.max_abs_error);
}

#[test]
fn recovers_curvy_pool_within_bps() {
    let big_k = 40_000_000_000_000_000_000_000u128; // 4e22, strong curvature
    let fee = 30_000u64; // 3%
    let samples = gen_samples(
        big_k,
        fee,
        &[
            1_000_000_000,
            50_000_000_000,
            100_000_000_000,
            1_000_000_000_000,
            2_000_000_000_000,
            3_000_000_000_000,
        ],
    );

    let r = fit_obric_form(&samples).unwrap();
    assert!(r.max_error_bps < 0.1, "max_error_bps = {}", r.max_error_bps);
}

#[test]
fn predicts_held_out_size() {
    let big_k = 100_000_000_000_000_000_000_000_000u128; // 1e26
    let fee = 500u64;
    let train = gen_samples(
        big_k,
        fee,
        &[
            1_000_000_000,
            10_000_000_000,
            100_000_000_000,
            1_000_000_000_000,
            3_000_000_000_000,
        ],
    );
    let r = fit_obric_form(&train).unwrap();

    // A size that was not in the training set should still be predicted accurately.
    let held = gen_samples(big_k, fee, &[700_000_000_000]);
    let predicted = predict(&held[0], r.params).unwrap();
    let actual = held[0].amount_out;
    let bps = (predicted as f64 - actual as f64).abs() / actual as f64 * 10_000.0;
    assert!(bps < 0.5, "held-out bps = {bps}");
}

#[test]
fn empty_samples_is_none() {
    assert!(fit_obric_form(&[]).is_none());
}
