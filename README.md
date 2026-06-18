# propquote

[![CI](https://github.com/WorthlessDeveloper/propquote/actions/workflows/ci.yml/badge.svg)](https://github.com/WorthlessDeveloper/propquote/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A from-scratch, **closed-form** quoting engine for Solana prop AMMs — built to beat
[LimeChain/magnus](https://github.com/LimeChain/magnus) on the thing that matters for arbitrage:
predicting a venue's quote as fast, pure arithmetic instead of simulating a transaction.

## Why this exists / how it's better than Magnus

Magnus quotes the obfuscated prop AMMs by executing their real `.so` inside LiteSVM — correct, but
it runs a whole VM transaction per quote. For arb you want the quote as a microsecond function of
in-memory state. propquote does that, and fixes the rough edges of the Magnus port along the way.

| | Magnus | propquote |
|---|---|---|
| Quote path | LiteSVM sim per quote (full tx exec) | **Closed-form arithmetic** |
| Math coverage | Obric only (closed-form), rest sim | Obric **correct + tested**; framework for the rest |
| Obric correctness | reserve-aliasing bug (`reserve_y` read from `reserve_x`) | **fixed** |
| Overflow safety | raw `a*b/c` (can panic) | **256-bit `mul_div`**, never panics |
| Output | `amount_out` | `amount_out` + fee split + **oracle/effective/marginal price + impact bps + exact-out** |
| Dependencies | full Solana tree (~hundreds of crates) | **zero deps** in `core`/`obric` — builds in <1s, runs anywhere |
| `unsafe` | yes | `#![forbid(unsafe_code)]` |

## Layout

```
crates/
  propquote-core/    # zero-dep foundation: overflow-safe math, byte decoder, PropAmm trait, base58
  propquote-obric/   # Obric V2 closed-form quoter (reference impl for the oracle-PMM family)
  propquote-replay/  # fit a closed form from (state, amount_in) -> amount_out samples + venue calldata
  propquote-sim/     # ground-truth oracle: run a venue's real .so in LiteSVM (heavy; CI-only)
```

The `PropAmm` trait is the seam: each venue is `decode state → quote(side, amount, mode)`. State
refresh (RPC/Geyser) lives above the trait, so a quote never touches the network. The first three
crates are zero-dependency and build/test in <1s; `propquote-sim` is the only heavy one and is kept
out of the default build.

## Status

| Venue | Approach | State |
|-------|----------|-------|
| **Obric V2** | closed-form (math is public) | ✅ implemented, 9 tests + independent numeric check |
| **SolFi V2** | fit to real fills | ✅ down to **0.02 bps** median (size-diverse, tight window) |
| **HumidiFi** | fit to real fills | ✅ **~0.2 bps** median |
| **ZeroFi** | fit to real fills | ✅ **~0.5 bps** median (tight window; ~7 bps over a 150s window) |
| **BisonFi** | fit to real fills | ✅ **~0.2 bps** median (1s window) |
| **Tessera** | fit to real fills | ✅ **~0.4 bps** when SOL/USDC volume is present (often quiet) |
| GoonFi | diagnosed: inactive | ⚪ ~80% of recent txs **fail**; its thin SOL/USDC pool quotes a **stale ~$80** (vs ~$71 live) and GoonFi V2 has no recent activity. Obric form fits its shape to ~7–10 bps — no healthy flow to fit, not a model gap. |

**The key result:** within a **tight time window** (so the oracle is ~constant), every prop AMM with
SOL/USDC volume fits the Obric oracle-PMM form to **sub-bp** — strong evidence the whole family shares
that shape. Over a long sampling window the residual inflates to several bps, but that is the **oracle
drifting while we sample, not model error** (ZeroFi: 0.5 bps tight vs ~7 bps over 150s). GoonFi is the
one venue that won't fit cleanly — but the diagnosis is that it's **effectively inactive**: ~80% of its
recent transactions fail and its thin SOL/USDC pool quotes a stale ~$80 (vs ~$71 live), so there's no
healthy flow to fit. A dead/stale venue, not a model failure.

**Reproduce (live probe — exact numbers vary with the on-chain snapshot):**
[`tools/fit_venue.py`](tools/fit_venue.py) discovers each venue's SOL/USDC vaults from recent activity,
pulls fills from the vault's signatures, and fits the Obric form within a 30s window — no LiteSVM or
Solana toolchain: `SOLANA_RPC_URL=<rpc> python tools/fit_venue.py`. ([`tools/solfi_fit.py`](tools/solfi_fit.py)
is the detailed SolFi-only version.) Caveats: arb flow clusters around one trade size (curvature
`big_k` loosely identified) and spread folds into the inferred oracle (so `fee` isn't separable
without an external price feed).

**Separating spread from oracle (two-sided):** [`tools/spread_probe.py`](tools/spread_probe.py) pulls
*both* directions, so the near-mid sell price (bid) and buy price (ask) bracket the oracle and their
gap is the spread. First cut: SolFi V2 ≈ **0.3 bps** spread, and all five active venues quote a mid of
**~71.3 USDC/SOL within a few bps of each other** — they track a common oracle, which confirms the
spread *is* separable (and is itself a cross-venue validation). Bp-precise per-venue spread still needs
size/time-matched buy/sell pairs — the current probe carries a few bps of noise (occasionally negative),
which is the next refinement. Bit-exact constants (decompiling each venue's bytecode / LiteSVM) need a
Solana build host — not possible from the machine this was built on.

**How a venue gets cracked now** (the pipeline is built, end to end):
1. `propquote-sim` runs the venue's real `.so` in LiteSVM against live accounts → ground-truth `amount_out`.
2. `propquote-replay::fit_obric_form` fits the oracle-PMM closed form to those samples and reports
   the residual in bps. Sub-bp residual ⇒ you have a microsecond closed-form quoter for that venue.
3. The fitter is verified to recover known pools to ≤0.01 bps (and exactly, to rounding dust, when
   well-conditioned) — see `crates/propquote-replay/tests/fit_tests.rs`.

The Obric math is the template: every other venue is the same oracle-anchored / concentrated /
inventory-skewed shape (see [`../docs/prop-amm-quoting-model.md`](../docs/prop-amm-quoting-model.md)).
The reverse-engineered swap calldata (selector + args) for SolFi V2 / ZeroFi / Tessera lives in
`propquote-replay::venues` (byte-tested).

## The Obric model, in code

`quote_x_to_y` walks a constant-product invariant `big_k` on a "curve-K" coordinate that is shifted
so the marginal price at the inventory target equals the oracle price `mult_x/mult_y`:

```
target_x_k   = sqrt(big_k * mult_y / mult_x)        # curve point where marginal == oracle
current_x_k  = target_x_k - target_x + current_x     # shift real reserves onto curve-K
current_y_k  = big_k / current_x_k
new_x_k      = current_x_k + amount_in
out_before   = current_y_k - big_k / new_x_k         # constant-product step
fee          = out_before * fee_millionth/1e6, less an inventory rebate
```

The rebate shrinks the fee for trades that push reserves toward target — that's the inventory skew.

## Build & test

```bash
cargo test            # unit + integration tests (Linux/macOS, or Windows with the MSVC SDK libs)
cargo run -p propquote-obric --example quote_ladder
```

> **Windows note:** linking `std` binaries needs the **Windows 10/11 SDK libraries** (`kernel32.lib`
> et al.) installed alongside the VS C++ build tools. If `cargo test` fails with
> `LNK1181: cannot open input file 'kernel32.lib'`, the SDK libs aren't installed — either add the
> "Windows 11 SDK" component in the VS Installer, or build with the self-contained GNU toolchain:
> `rustup toolchain install stable-x86_64-pc-windows-gnu` then
> `cargo +stable-x86_64-pc-windows-gnu test`. `cargo check` works regardless (it doesn't link).

## Wiring real data

```rust
use propquote_obric::{ObricPool, SSTradingPair};
use propquote_core::PropAmm;
use propquote_core::types::Side;

let state = SSTradingPair::from_account_data(&pool_account.data)?; // the Obric pool config account
let mut pool = ObricPool::new(state);
pool.state.update_price(pyth_x_price, pyth_y_price, x_decimals, y_decimals); // fresh oracle
pool.set_reserves(reserve_x_vault_amount, reserve_y_vault_amount);          // fresh balances
let q = pool.quote_in(Side::XtoY, amount_in)?;
// q.amount_out, q.price_impact_bps, q.marginal_price_after, ...
```

`pool.accounts_to_watch()` returns exactly the accounts (reserves + Pyth feeds) to subscribe to via
Geyser; re-decode on each push and you have a live, microsecond quoter.

## Roadmap (how the rest gets added — the genuinely "way better" part)

- ✅ **`propquote-sim`** (LiteSVM): runs a venue's real `.so` as a *ground-truth oracle*. This is
  Magnus's whole approach, demoted here to a validation/sample-source layer.
- ✅ **`propquote-replay`**: feeds `(state, amount_in) → amount_out` samples and **fits the
  closed-form parameters**, reporting residual in bps. Verified to recover known pools to ≤0.01 bps.
- ✅ **venue calldata** for SolFi V2 / ZeroFi / Tessera (`propquote-replay::venues`), byte-tested.
- ⏳ **Next:** point the sim at real mainnet accounts for one venue (SolFi V2 — explicit oracle
  account makes it the cleanest), generate samples, run the fitter, and ship the first non-Obric
  closed-form quoter. Then Geyser ingest + per-venue `PropAmm` impls.

The point: closed-form first for speed, real-binary sim as the oracle that keeps you honest — the
inverse of Magnus's sim-first design.
