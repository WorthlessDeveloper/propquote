# propquote

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
```

The `PropAmm` trait is the seam: each venue is `decode state → quote(side, amount, mode)`. State
refresh (RPC/Geyser) lives above the trait, so a quote never touches the network.

## Status

| Venue | Approach | State |
|-------|----------|-------|
| **Obric V2** | closed-form (math is public) | ✅ implemented, 9 tests + independent numeric check |
| SolFi V2 / ZeroFi / Tessera / HumidiFi / GoonFi / BisonFi | closed-form via fit-against-sim | ⏳ planned (see roadmap) |

The Obric math is the template: every other venue is the same oracle-anchored / concentrated /
inventory-skewed shape (see [`../docs/prop-amm-quoting-model.md`](../docs/prop-amm-quoting-model.md)).

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

1. **`propquote-sim`** (feature-gated, LiteSVM): runs a venue's real `.so` as a *ground-truth oracle*.
   This is Magnus's whole approach, demoted here to a validation/fallback layer.
2. **`propquote-replay`**: feed `(account state, amount_in) → amount_out` samples (from the sim or
   from historical fills) and **fit each venue's closed-form parameters** until bit-exact. This is
   how SolFi/ZeroFi/Tessera/HumidiFi graduate from "sim only" to "closed-form fast path", with the
   sim kept as a continuous correctness check (operators reparametrize).
3. **Geyser ingest + per-venue `PropAmm` impls** following the Obric template.

The point: closed-form first for speed, real-binary sim as the oracle that keeps you honest — the
inverse of Magnus's sim-first design.
