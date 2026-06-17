//! Demo: quote a size ladder against a synthetic Obric V2 stable pool and print arb-grade metrics.
//!
//! Run with:  `cargo run -p propquote-obric --example quote_ladder`
//!
//! In production you'd build the pool from real account data:
//!   let state = SSTradingPair::from_account_data(&rpc_account.data)?;   // the pool config
//!   let mut pool = ObricPool::new(state);
//!   pool.state.update_price(pyth_x, pyth_y, x_dec, y_dec);              // fresh oracle
//!   pool.set_reserves(vault_x_amount, vault_y_amount);                 // fresh vault balances

use propquote_core::types::Side;
use propquote_core::PropAmm;
use propquote_obric::{ObricPool, SSTradingPair};

fn main() {
    // A concentrated 1:1 stable pool (think USDC/USDT), 6 decimals, ~5M each side.
    let unit = 1_000_000u64; // 6 decimals
    let depth = 5_000_000 * unit; // 5,000,000.000000

    let mut state = SSTradingPair::default();
    state.is_initialized = true;
    state.target_x = depth;
    // Concentration: put target_x_k far above target_x so the curve is very flat near the peg.
    let target_x_k: u128 = 500_000_000_000_000; // 5e14
    state.big_k = target_x_k * target_x_k; // mults are 1:1, so target_x_k = sqrt(big_k)
    state.fee_millionth = 100; // 1 bp
    state.rebate_percentage = 0;
    state.protocol_fee_share_thousandth = 200; // 20% of fees to protocol
    state.update_price(1000, 1000, 6, 6); // Pyth prices scaled ×1000 -> mult_x = mult_y = 1000

    let mut pool = ObricPool::new(state);
    pool.set_reserves(depth, depth);

    println!("Obric V2 — synthetic USDC/USDT pool, 5,000,000 each side, 1 bp fee\n");
    println!(
        "{:>14}  {:>16}  {:>12}  {:>12}  {:>10}",
        "size (X in)", "out (Y)", "eff price", "impact bps", "fee (Y)"
    );
    println!("{}", "-".repeat(72));

    for usd in [1_000u64, 10_000, 100_000, 1_000_000, 4_000_000] {
        let amount = usd * unit;
        match pool.quote_in(Side::XtoY, amount) {
            Ok(q) => println!(
                "{:>14}  {:>16}  {:>12.6}  {:>12.4}  {:>10}",
                amount, q.amount_out, q.effective_price, q.price_impact_bps, q.fee
            ),
            Err(e) => println!("{:>14}  ERROR: {}", amount, e),
        }
    }

    // Exact-out: how much X must I sell to receive exactly 250,000 USDT?
    let want = 250_000 * unit;
    match pool.quote_out(Side::XtoY, want) {
        Ok(q) => println!(
            "\nexact-out: to receive {} Y, sell {} X (impact {:.4} bps)",
            want, q.amount_in, q.price_impact_bps
        ),
        Err(e) => println!("\nexact-out error: {e}"),
    }
}
