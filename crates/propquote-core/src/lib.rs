//! `propquote-core` — the dependency-free foundation for closed-form prop-AMM quoting.
//!
//! Three pieces, deliberately small and panic-free:
//! - [`math`]: overflow-safe `mul_div` (full 256-bit intermediate) and integer `isqrt`.
//! - [`decode`]: a bounds-checked little-endian cursor for parsing raw on-chain account bytes.
//! - [`amm`]: the [`amm::PropAmm`] trait every venue implements, plus the rich [`types::QuoteResult`].
//!
//! Nothing here depends on `solana-sdk`, `anchor`, or the network — so a quote is pure arithmetic
//! and the whole crate builds and tests in well under a second.

pub mod amm;
pub mod bs58;
pub mod decode;
pub mod math;
pub mod types;

pub use amm::PropAmm;
pub use types::{Pubkey, QuoteError, QuoteResult, Side, SwapMode};
