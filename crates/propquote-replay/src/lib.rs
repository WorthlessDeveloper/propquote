//! `propquote-replay` — turn ground-truth swap observations into a fast closed-form quoter.
//!
//! The workflow for cracking an obfuscated venue (SolFi, ZeroFi, Tessera, …):
//!
//! 1. Collect [`Sample`]s of `(reserves, oracle, amount_in) -> amount_out`. The ground truth comes
//!    from `propquote-sim` (the venue's real `.so` run in LiteSVM) or from historical on-chain fills.
//! 2. [`fit_obric_form`] fits an oracle-PMM closed form to the samples and reports the residual.
//! 3. If the residual is small, you now have a microsecond closed-form quoter for that venue; if
//!    not, the venue's functional form differs and needs a different model — the residual tells you.
//!
//! The Obric form is the default model because every observed prop AMM is the same family
//! (oracle-anchored, concentrated, inventory-skewed). [`venues`] holds the reverse-engineered
//! swap calldata each venue's program expects, used by the sim to drive the real program.

pub mod fit;
pub mod sample;
pub mod venues;

pub use fit::{fit_obric_form, predict, FitResult, ObricParams};
pub use sample::Sample;
