//! Closed-form quoter for **Obric V2** (`obriQD1zbpyLz95G5n7nJe6a4DPjpFwa5XYPoNm113y`).
//!
//! Obric is the one Solana prop AMM whose pricing is public, which makes it the reference
//! implementation for the whole oracle-PMM family (SolFi, ZeroFi, Tessera, HumidiFi all share the
//! shape: oracle reference price + concentrated curve + inventory skew).
//!
//! Pricing model (reconstructed from the on-chain `SSTradingPair` state):
//! - An oracle price `p = mult_x / mult_y` anchors the curve.
//! - A constant-product invariant `big_k` on a shifted "curve-K" coordinate gives price impact,
//!   concentrated around the inventory `target_x`.
//! - A fee (`fee_millionth`) with an inventory **rebate** that shrinks the fee for trades pushing
//!   reserves back toward target — i.e. the inventory skew.
//!
//! Versus the Magnus port this implementation: fixes the reserve-aliasing bug, never panics on
//! overflow (256-bit `mul_div`), supports exact-out, and returns marginal price + impact.

pub mod quote;
pub mod state;

pub use quote::{ObricPool, OBRIC_V2_PROGRAM_ID};
pub use state::SSTradingPair;
