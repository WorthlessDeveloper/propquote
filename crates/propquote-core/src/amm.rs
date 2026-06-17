//! The `PropAmm` trait every venue implements.
//!
//! The contract is intentionally narrow: hold decoded state + current reserves, and answer quotes
//! as pure arithmetic. State refresh (RPC/Geyser) lives outside; this layer never touches the
//! network, which is what makes a quote a microsecond function instead of a millisecond round-trip.

use crate::types::{Pubkey, QuoteError, QuoteResult, Side, SwapMode};

pub trait PropAmm {
    /// Human-readable venue label, e.g. `"ObricV2"`.
    fn label(&self) -> &'static str;

    /// The on-chain program id for this venue.
    fn program_id(&self) -> Pubkey;

    /// Accounts whose data must be kept fresh for quotes to be correct (reserves, oracle feeds,
    /// price/param accounts). Subscribe to these via Geyser and re-decode on change.
    fn accounts_to_watch(&self) -> Vec<Pubkey>;

    /// Quote a swap. `amount` is interpreted per `mode` (exact-in spends it, exact-out targets it).
    fn quote(&self, side: Side, amount: u64, mode: SwapMode) -> Result<QuoteResult, QuoteError>;

    /// Convenience: exact-in quote.
    fn quote_in(&self, side: Side, amount_in: u64) -> Result<QuoteResult, QuoteError> {
        self.quote(side, amount_in, SwapMode::ExactIn)
    }

    /// Convenience: exact-out quote.
    fn quote_out(&self, side: Side, amount_out: u64) -> Result<QuoteResult, QuoteError> {
        self.quote(side, amount_out, SwapMode::ExactOut)
    }
}
