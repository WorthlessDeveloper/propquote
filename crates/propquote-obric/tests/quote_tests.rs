//! Behavioural tests for the Obric V2 closed-form quoter, using hand-computed vectors.

use propquote_core::types::{QuoteError, Side, SwapMode};
use propquote_core::PropAmm;
use propquote_obric::{ObricPool, SSTradingPair};

/// A simple symmetric stable pool: mults 1:1, target == current_x, big_k = 1e14
/// so target_x_k = sqrt(1e14) = 1e7.
fn stable_pool(fee_millionth: u64, rebate_percentage: u64, protocol_share_thousandth: u64) -> ObricPool {
    let mut s = SSTradingPair::default();
    s.is_initialized = true;
    s.mult_x = 1_000;
    s.mult_y = 1_000;
    s.big_k = 100_000_000_000_000; // 1e14
    s.target_x = 1_000_000;
    s.fee_millionth = fee_millionth;
    s.rebate_percentage = rebate_percentage;
    s.protocol_fee_share_thousandth = protocol_share_thousandth;
    let mut p = ObricPool::new(s);
    p.set_reserves(1_000_000, 1_000_000);
    p
}

#[test]
fn exact_in_no_fee_matches_hand_calc() {
    // current_x_k = 1e7, current_y_k = 1e7, new_x_k = 1.1e7, new_y_k = floor(1e14/1.1e7) = 9_090_909
    // out_before_fee = 1e7 - 9_090_909 = 909_091, fee 0 -> 909_091.
    let p = stable_pool(0, 0, 0);
    let q = p.quote(Side::XtoY, 1_000_000, SwapMode::ExactIn).unwrap();
    assert_eq!(q.amount_out, 909_091);
    assert_eq!(q.fee, 0);
    assert_eq!(q.amount_in, 1_000_000);
}

#[test]
fn exact_in_with_fee_no_rebate() {
    // fee_millionth = 10_000 (1%). fee_before = floor(909_091 * 10_000 / 1e6) = 9_090.
    // current_x == target_x so deficit = 0 -> rebate_ratio 0 -> no rebate.
    // out_after = 909_091 - 9_090 = 900_001. protocol share 0 -> all lp.
    let p = stable_pool(10_000, 100, 0);
    let q = p.quote(Side::XtoY, 1_000_000, SwapMode::ExactIn).unwrap();
    assert_eq!(q.amount_out, 900_001);
    assert_eq!(q.fee, 9_090);
    assert_eq!(q.protocol_fee, 0);
    assert_eq!(q.lp_fee, 9_090);
}

#[test]
fn protocol_fee_split() {
    // Same as above but protocol takes 300/1000 of the fee: floor(9090 * 300 / 1000) = 2727.
    let p = stable_pool(10_000, 0, 300);
    let q = p.quote(Side::XtoY, 1_000_000, SwapMode::ExactIn).unwrap();
    assert_eq!(q.fee, 9_090);
    assert_eq!(q.protocol_fee, 2_727);
    assert_eq!(q.lp_fee, 9_090 - 2_727);
}

#[test]
fn rebate_applies_when_trade_reduces_x_deficit() {
    // Pool is short on X: target_x (1.5M) > current_x (1M), a 500k deficit. Selling X *adds* X,
    // pushing inventory toward target, so the fee is rebated. Y reserve is ample to fill the trade.
    let mut s = SSTradingPair::default();
    s.is_initialized = true;
    s.mult_x = 1_000;
    s.mult_y = 1_000;
    s.big_k = 100_000_000_000_000;
    s.target_x = 1_500_000; // deficit of 500_000 below current_x
    s.fee_millionth = 10_000; // 1%
    s.rebate_percentage = 100; // full rebate eligible
    s.protocol_fee_share_thousandth = 0;
    let mut p = ObricPool::new(s);
    p.set_reserves(1_000_000, 5_000_000);

    // Selling exactly the deficit (500_000) X: min(input, deficit) = 500_000 -> rebate_ratio 100%.
    // The whole fee is rebated -> fee == 0. (Verified independently: out_before_fee = 526_315.)
    let q = p.quote(Side::XtoY, 500_000, SwapMode::ExactIn).unwrap();
    assert_eq!(q.fee, 0, "full inventory rebate should zero the fee");
    assert_eq!(q.amount_out, 526_315);
}

#[test]
fn exact_out_inverts_exact_in() {
    let p = stable_pool(2_000, 0, 0); // 2 bps-ish fee
    let target_out = 500_000u64;
    let q = p.quote(Side::XtoY, target_out, SwapMode::ExactOut).unwrap();

    // The solved input must actually deliver at least the target...
    assert!(q.amount_out >= target_out, "solved input underfills");
    // ...and be minimal: one unit less must underfill.
    let less = p.quote(Side::XtoY, q.amount_in - 1, SwapMode::ExactIn).unwrap();
    assert!(less.amount_out < target_out, "input was not minimal");
}

#[test]
fn metrics_are_sane() {
    let p = stable_pool(2_000, 0, 0);
    let q = p.quote(Side::XtoY, 100_000, SwapMode::ExactIn).unwrap();

    // 1:1 oracle for a stable pair.
    assert!((q.oracle_price - 1.0).abs() < 1e-9);
    // Taker pays spread+impact, so effective < oracle and impact is positive.
    assert!(q.effective_price < q.oracle_price);
    assert!(q.price_impact_bps > 0.0);
    // Price moves against the taker: marginal price after < before (less Y per X available).
    assert!(q.marginal_price_after < q.marginal_price_before);
}

#[test]
fn both_directions_quote() {
    let p = stable_pool(2_000, 0, 0);
    let xy = p.quote(Side::XtoY, 100_000, SwapMode::ExactIn).unwrap();
    let yx = p.quote(Side::YtoX, 100_000, SwapMode::ExactIn).unwrap();
    // Symmetric pool -> symmetric outputs.
    assert_eq!(xy.amount_out, yx.amount_out);
}

#[test]
fn zero_amount_rejected() {
    let p = stable_pool(0, 0, 0);
    assert_eq!(p.quote(Side::XtoY, 0, SwapMode::ExactIn), Err(QuoteError::ZeroAmount));
}

#[test]
fn program_id_and_watched_accounts() {
    let p = stable_pool(0, 0, 0);
    // Obric V2 program id decodes to 32 bytes and is non-zero.
    assert_ne!(p.program_id(), [0u8; 32]);
    assert_eq!(p.accounts_to_watch().len(), 4);
    assert_eq!(p.label(), "ObricV2");
}
