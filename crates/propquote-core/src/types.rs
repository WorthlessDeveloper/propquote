//! Shared types: pubkeys, swap direction/mode, the error enum, and the rich quote result.

use core::fmt;

/// A Solana account address as raw bytes. We deliberately avoid pulling in `solana-sdk` just for
/// a 32-byte array; [`crate::bs58`] converts to/from the usual base58 string form.
pub type Pubkey = [u8; 32];

/// Direction of a swap relative to a pool's `(x, y)` token ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Sell token X, receive token Y.
    XtoY,
    /// Sell token Y, receive token X.
    YtoX,
}

impl Side {
    pub fn flip(self) -> Side {
        match self {
            Side::XtoY => Side::YtoX,
            Side::YtoX => Side::XtoY,
        }
    }
}

/// Whether `amount` is the exact input to spend or the exact output to receive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapMode {
    ExactIn,
    ExactOut,
}

/// Non-panicking error type for decoding and quoting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteError {
    /// The requested amount was zero.
    ZeroAmount,
    /// The pool cannot fill the requested size at any price.
    InsufficientLiquidity,
    /// An intermediate computation exceeded `u128`.
    MathOverflow,
    /// A subtraction would have gone negative (e.g. inconsistent reserves vs. target).
    Underflow,
    /// The provided mint is not one of the pool's two tokens.
    UnsupportedMint,
    /// An iterative solve (exact-out) did not converge within its budget.
    NotConverged,
    /// A byte buffer was shorter than the layout required.
    ShortBuffer,
    /// Field bytes were structurally invalid (bad bool, bad base58, etc).
    InvalidData,
}

impl fmt::Display for QuoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            QuoteError::ZeroAmount => "amount must be non-zero",
            QuoteError::InsufficientLiquidity => "insufficient liquidity to fill",
            QuoteError::MathOverflow => "arithmetic overflow",
            QuoteError::Underflow => "arithmetic underflow",
            QuoteError::UnsupportedMint => "mint is not part of this pool",
            QuoteError::NotConverged => "exact-out solve did not converge",
            QuoteError::ShortBuffer => "account buffer too short for layout",
            QuoteError::InvalidData => "structurally invalid field data",
        };
        f.write_str(s)
    }
}

impl std::error::Error for QuoteError {}

/// The result of a quote. Far richer than a bare `amount_out`: everything an arbitrageur needs to
/// size a trade and compare against the true market is here, computed in the same pass.
///
/// Prices are expressed as **output units per input unit** for the requested side, so
/// `effective_price` and `oracle_price` are directly comparable and `price_impact_bps` is the
/// signed cost (positive = you received less than the oracle mid implied).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QuoteResult {
    pub amount_in: u64,
    pub amount_out: u64,
    /// Total fee charged (protocol + lp), denominated in the output token.
    pub fee: u64,
    pub protocol_fee: u64,
    pub lp_fee: u64,
    /// Oracle mid for this side, output-per-input.
    pub oracle_price: f64,
    /// Realized price for this fill, `amount_out / amount_in`.
    pub effective_price: f64,
    /// Instantaneous (marginal) price before the trade.
    pub marginal_price_before: f64,
    /// Instantaneous (marginal) price after the trade — the new top-of-book.
    pub marginal_price_after: f64,
    /// `(oracle - effective) / oracle * 1e4`. Positive = taker pays spread+impact.
    pub price_impact_bps: f64,
}
