//! Bring-up: run real SolFi V2 swaps in LiteSVM against live mainnet accounts, emit a ground-truth
//! size-ladder, and fit the Obric closed form to it.
//!
//! Driven by the `fit.yml` workflow, which dumps the program `.so` and fetches the accounts (with
//! your RPC) into `CFG_DIR` before running this. Account files are the JSON from
//! `solana account <PUBKEY> --output json`.
//!
//! Env:
//! - `CFG_DIR`  directory holding `solfi_v2.so` and `<role>.json` account files (default `cfg/fit`).
//! - `SLOT`     slot to warp the VM to so the oracle snapshot is considered fresh (default 0 = none).
//!
//! This is intentionally verbose: run #1 is a debugging run. The headline output is the ladder
//! (`amount_in -> amount_out` straight from the venue's own bytecode); the fit is best-effort.

use propquote_core::types::Side;
use propquote_replay::{fit_obric_form, venues, Sample};
use propquote_sim::{account_json::read_account_file, GroundTruthSvm};
use solana_instruction::AccountMeta;
use solana_sdk::pubkey::Pubkey;

// Pool: SolFi V2 SOL/USDC (addresses from LimeChain/magnus `cfg/payloads/pmms.json`; edit if stale).
const PROGRAM: &str = "SV2EYYJyRz2YhfXwXnhNAevDEui5Q6yrfyo13WtupPF";
const MARKET: &str = "65ZHSArs5XxPseKQbB1B4r16vDxMWnCxHMzogDAqiDUc";
const BASE_VAULT: &str = "CRo8DBwrmd97DJfAnvCv96tZPL5Mktf2NZy2ZnhDer1A";
const QUOTE_VAULT: &str = "GhFfLFSprPpfoRaWakPMmJTMJBHuz6C694jYwxy2dAic";
const GLOBAL_CONFIG: &str = "FmxXDSR9WvpJTCh738D1LEDuhMoA8geCtZgHb3isy7Dp";
const ORACLE: &str = "2ny7eGyZCoeEVTkNLf5HcnJFBKkyA4p4gcrtb3b8y8ou";
const BASE_MINT: &str = "So11111111111111111111111111111111111111112"; // WSOL, 9 decimals
const QUOTE_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"; // USDC, 6 decimals
const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const INSTRUCTIONS_SYSVAR: &str = "Sysvar1nstructions1111111111111111111111111";

fn pk(s: &str) -> Pubkey {
    s.parse().unwrap_or_else(|e| panic!("bad pubkey {s}: {e}"))
}

fn main() {
    let cfg_dir = std::env::var("CFG_DIR").unwrap_or_else(|_| "cfg/fit".to_string());
    let slot: u64 = std::env::var("SLOT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let program = pk(PROGRAM);
    let mut svm = GroundTruthSvm::new();
    if slot > 0 {
        svm.warp_to_slot(slot);
        println!("warped VM to slot {slot}");
    }

    // 1. Load the venue program and the live accounts it reads.
    if let Err(e) = svm.load_program(program, format!("{cfg_dir}/solfi_v2.so")) {
        eprintln!("FATAL: load program: {e}");
        std::process::exit(1);
    }
    let accounts: [(&str, &str); 7] = [
        ("market", MARKET),
        ("base_vault", BASE_VAULT),
        ("quote_vault", QUOTE_VAULT),
        ("global_config", GLOBAL_CONFIG),
        ("oracle", ORACLE),
        ("base_mint", BASE_MINT),
        ("quote_mint", QUOTE_MINT),
    ];
    for (name, addr) in accounts {
        match read_account_file(format!("{cfg_dir}/{name}.json")) {
            Ok(acc) => {
                println!(
                    "loaded {name:14} {addr}  ({} bytes, owner {})",
                    acc.data.len(),
                    acc.owner
                );
                if let Err(e) = svm.set_account(pk(addr), acc) {
                    eprintln!("FATAL: set {name}: {e}");
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("FATAL: read {name}: {e}");
                std::process::exit(1);
            }
        }
    }

    let base_reserve = svm.token_balance(pk(BASE_VAULT));
    let quote_reserve = svm.token_balance(pk(QUOTE_VAULT));
    println!("\nreserves: base(WSOL)={base_reserve}  quote(USDC)={quote_reserve}\n");

    // 2. Run a size ladder of WSOL -> USDC swaps against the real program. The range spans small
    // (oracle-priced) to large (up to a few % of reserves) so the curve's price impact is visible
    // and `big_k` is identifiable — tiny trades alone leave the curvature degenerate.
    let ladder_sol: [f64; 10] = [
        0.1, 1.0, 5.0, 10.0, 50.0, 100.0, 250.0, 500.0, 750.0, 1000.0,
    ];
    let mut samples: Vec<Sample> = Vec::new();

    println!(
        "{:>16}  {:>16}  {:>12}",
        "in (lamports)", "out (USDC u)", "px(USDC/SOL)"
    );
    println!("{}", "-".repeat(50));
    for sol in ladder_sol {
        let amount_in = (sol * 1_000_000_000.0) as u64;

        // Reset the wallet's token accounts to a clean state for a clean balance delta.
        let user_wsol = match svm.create_wallet_ata(pk(BASE_MINT), amount_in) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("ata WSOL: {e}");
                continue;
            }
        };
        let user_usdc = match svm.create_wallet_ata(pk(QUOTE_MINT), 0) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("ata USDC: {e}");
                continue;
            }
        };

        let metas = vec![
            AccountMeta::new(svm.wallet_pubkey(), true),
            AccountMeta::new(pk(MARKET), false),
            AccountMeta::new_readonly(pk(ORACLE), false),
            AccountMeta::new_readonly(pk(GLOBAL_CONFIG), false),
            AccountMeta::new(pk(BASE_VAULT), false),
            AccountMeta::new(pk(QUOTE_VAULT), false),
            AccountMeta::new(user_wsol, false),
            AccountMeta::new(user_usdc, false),
            AccountMeta::new_readonly(pk(BASE_MINT), false),
            AccountMeta::new_readonly(pk(QUOTE_MINT), false),
            AccountMeta::new_readonly(pk(TOKEN_PROGRAM), false),
            AccountMeta::new_readonly(pk(TOKEN_PROGRAM), false),
            AccountMeta::new_readonly(pk(INSTRUCTIONS_SYSVAR), false),
        ];
        let data = venues::solfi_v2::swap_data(amount_in, 0, 0); // direction 0 = base -> quote

        match svm.simulate_swap(program, metas, data, user_usdc) {
            Ok(out) if out > 0 => {
                let px = out as f64 / 1e6 / sol;
                println!("{amount_in:>16}  {out:>16}  {px:>12.4}");
                samples.push(Sample {
                    current_x: base_reserve,
                    current_y: quote_reserve,
                    mult_x: 0, // filled after we infer the oracle scale, below
                    mult_y: 0,
                    target_x: base_reserve,
                    side: Side::XtoY,
                    amount_in,
                    amount_out: out,
                });
            }
            Ok(_) => println!("{amount_in:>16}  {:>16}  (zero out)", "-"),
            Err(e) => println!("{amount_in:>16}  swap reverted: {e}"),
        }
    }

    if samples.len() < 3 {
        eprintln!(
            "\nonly {} successful swaps — not enough to fit. See reverts above.",
            samples.len()
        );
        std::process::exit(samples.is_empty() as i32);
    }

    // 3. Infer the oracle scale from the smallest fill (effective price -> mult), then fit.
    let smallest = &samples[0];
    let ratio = smallest.amount_out as f64 / smallest.amount_in as f64;
    let mult_y: u64 = 1_000_000;
    let mult_x: u64 = (ratio * mult_y as f64).round().max(1.0) as u64;
    for s in &mut samples {
        s.mult_x = mult_x;
        s.mult_y = mult_y;
    }
    println!("\ninferred oracle scale: mult_x={mult_x} mult_y={mult_y} (raw out/in = {ratio:.9})");

    match fit_obric_form(&samples) {
        Some(r) => {
            println!("\n=== FIT (Obric form) ===");
            println!("big_k         = {}", r.params.big_k);
            println!("fee_millionth = {}", r.params.fee_millionth);
            println!("samples       = {}", r.samples);
            println!("max abs error = {} USDC units", r.max_abs_error);
            println!("max error     = {:.5} bps", r.max_error_bps);
            if r.max_error_bps < 5.0 {
                println!("\n✅ SolFi V2 is well-approximated by the Obric form at this state.");
            } else {
                println!(
                    "\n⚠️ residual is high — SolFi's curve may differ from the Obric form; \
                          inspect the ladder shape."
                );
            }
        }
        None => eprintln!("fit returned None"),
    }
}
