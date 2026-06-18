# Bit-exact handoff (item #1) — for a Solana build host

This is the one piece that couldn't be done on the original build machine (no working linker, full
disk, and the Solana 3.0 tree won't compile there). On **Linux/macOS with the Solana toolchain** it's
straightforward. Goal: get **bit-exact ground truth** by running each venue's *real* program, then
(optionally) decompile its bytecode for the literal constants.

Everything below already exists in this repo — `crates/propquote-sim` (LiteSVM harness +
`fit_solfi_v2` bin), the swap calldata in `crates/propquote-replay/src/venues.rs`, and the account
layouts/selectors in [`magnus-teardown.md`](magnus-teardown.md). You're un-blocking it, not building
from scratch.

## Prereqs
- Rust ≥ 1.85 (the Solana 3.0 deps need edition 2024)
- Solana CLI: `sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"`
- A mainnet RPC URL

## Step 1 — un-exclude the sim crate and fix the dependency conflict
`propquote-sim` is currently `exclude`d in the root `Cargo.toml` (it didn't build on the origin
machine). Add it back to `members`, then resolve the one known conflict: **two `solana-hash` versions
(3.0.0 vs 4.4.0)** pulled in via `solana-shred-version 3.0.1`.

```bash
cargo tree -p propquote-sim -i solana-hash      # confirm the duplicate
```

Fix, easiest first:
1. **Bump the stack** — `litesvm = "0.12"` (from 0.8.2) with matching newer `solana-sdk`
   (`4.x`) / `solana-instruction` / `solana-compute-budget`. The newer set unifies on one
   `solana-hash` and is the most likely one-shot fix.
2. If you must stay on the pinned set, force a single hash: pin `solana-shred-version` to a build
   that uses `solana-hash 3.x`, or add `solana-hash` as a direct dep at the version the rest of the
   tree uses, then `cargo update`.

`crates/propquote-sim/src/lib.rs` mirrors the known-good litesvm 0.8.2 API; if you bump to 0.12,
expect a few signature tweaks (it's ~180 lines).

## Step 2 — dump programs + fetch live accounts
```bash
RPC=<your-rpc>
mkdir -p cfg/fit
solana program dump SV2EYYJyRz2YhfXwXnhNAevDEui5Q6yrfyo13WtupPF cfg/fit/solfi_v2.so --url "$RPC"
for name_addr in \
  market:65ZHSArs5XxPseKQbB1B4r16vDxMWnCxHMzogDAqiDUc \
  base_vault:CRo8DBwrmd97DJfAnvCv96tZPL5Mktf2NZy2ZnhDer1A \
  quote_vault:GhFfLFSprPpfoRaWakPMmJTMJBHuz6C694jYwxy2dAic \
  global_config:FmxXDSR9WvpJTCh738D1LEDuhMoA8geCtZgHb3isy7Dp \
  oracle:2ny7eGyZCoeEVTkNLf5HcnJFBKkyA4p4gcrtb3b8y8ou \
  base_mint:So11111111111111111111111111111111111111112 \
  quote_mint:EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v ; do
  n=${name_addr%%:*}; a=${name_addr##*:}
  solana account "$a" --output json --url "$RPC" > "cfg/fit/$n.json"
done
```

## Step 3 — run the real program and get bit-exact output
```bash
SLOT=$(solana slot --url "$RPC")
CFG_DIR=cfg/fit SLOT=$SLOT cargo run -p propquote-sim --bin fit_solfi_v2
```
This loads the real `.so` + live accounts into LiteSVM, runs a WSOL→USDC size ladder, and prints the
**venue's own bytecode output** per size. Likely first-run snags (all in the swap path, not the
harness): oracle staleness vs the warped slot, or the SPL-token program not preloaded — both are
small fixes, and the bin prints each loaded account + per-size revert reason.

## Step 4 — close the loop (this is the "bit-exact" validation we couldn't run)
- Feed the ladder to `propquote_replay::fit_obric_form` and confirm the closed form reproduces the
  real `.so` output **to the unit** (for Obric it should be exact; for the others the residual is the
  true model gap, with no oracle-drift noise because it's all one frozen state).
- Replicate Steps 2–3 for ZeroFi / Tessera / HumidiFi / BisonFi using the account layouts + selectors
  in [`magnus-teardown.md`](magnus-teardown.md) (HumidiFi also needs the XOR'd calldata — decoded
  there). This gives controlled, single-state ladders → clean curvature + spread, fixing the two
  precision caveats from the historical-fills approach.

## Step 5 — decompile for literal constants (optional, deepest)
```bash
solana program dump <PROGRAM_ID> prog.so
llvm-objdump -d prog.so          # or a dedicated sBPF disassembler / Ghidra w/ an sBPF spec
```
Find the swap handler by the instruction selector (SolFi `0x07`, Tessera `0x10`, ZeroFi disc `0x06`,
HumidiFi `0x04/0x0f/0x14`, GoonFi/BisonFi `0x02` — all in `magnus-teardown.md`), then read the
fixed-point arithmetic and guard thresholds. That yields the exact `fee` / `concentration` / staleness
constants the behavioral fit can only approximate. Expect hand-written sBPF (no symbols) — the
account-offset diffing in [`reverse-engineering-playbook.md`](reverse-engineering-playbook.md) is how
you map state without an IDL.

---
**Bottom line for the dev:** Steps 1–4 turn the behavioral model into a bit-exact one on a real build
host; Step 5 is only needed if you want the literal source constants rather than a model that
predicts the output exactly.
