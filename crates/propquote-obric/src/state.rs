//! On-chain `SSTradingPair` account layout for Obric V2, plus a precise byte decoder.
//!
//! Layout mirrors the program account (Anchor: an 8-byte discriminator precedes the struct).
//! We read every field in order via [`Cursor`] so the layout is self-documenting and we don't
//! depend on `anchor`/`borsh`. Trailing fields we don't need for quoting (volume history, sslp
//! mints, padding) are skipped rather than stored.

use propquote_core::decode::Cursor;
use propquote_core::types::{Pubkey, QuoteError};

/// Decoded Obric V2 trading-pair config. The numeric fields drive [`crate::quote::ObricPool`].
#[derive(Clone, Debug, Default)]
pub struct SSTradingPair {
    pub is_initialized: bool,

    /// Pyth price-feed accounts for X and Y. Watch these for fresh prices.
    pub x_price_feed_id: Pubkey,
    pub y_price_feed_id: Pubkey,

    /// SPL token vaults holding the pool's X and Y reserves. Watch these for fresh balances.
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,

    pub mint_x: Pubkey,
    pub mint_y: Pubkey,

    /// Concentration parameter (curve steepness knob). Stored for completeness; `big_k` is the
    /// working invariant used by the quote math.
    pub concentration: u64,
    /// Constant-product invariant on curve-K.
    pub big_k: u128,
    /// Target inventory of X (the curve is concentrated around this point).
    pub target_x: u64,

    /// Oracle-derived value scalers: `value_x = amount_x * mult_x`. Set by [`Self::update_price`].
    pub mult_x: u64,
    pub mult_y: u64,

    /// Fee in millionths of the output (e.g. `2_000` = 2 bps).
    pub fee_millionth: u64,
    /// How much of the fee is rebated for inventory-improving trades (percent, 0..=100).
    pub rebate_percentage: u64,
    /// Protocol's share of the (post-rebate) fee, in thousandths.
    pub protocol_fee_share_thousandth: u64,
}

impl SSTradingPair {
    pub const DISCRIMINATOR_LEN: usize = 8;

    /// Decode from a full account buffer (including the 8-byte Anchor discriminator).
    pub fn from_account_data(data: &[u8]) -> Result<Self, QuoteError> {
        if data.len() < Self::DISCRIMINATOR_LEN {
            return Err(QuoteError::ShortBuffer);
        }
        Self::from_bytes(&data[Self::DISCRIMINATOR_LEN..])
    }

    /// Decode from the struct bytes (discriminator already stripped).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, QuoteError> {
        let mut c = Cursor::new(bytes);

        let is_initialized = c.read_bool()?;
        let x_price_feed_id = c.read_pubkey()?;
        let y_price_feed_id = c.read_pubkey()?;
        let reserve_x = c.read_pubkey()?;
        let reserve_y = c.read_pubkey()?;
        let _protocol_fee_x = c.read_pubkey()?;
        let _protocol_fee_y = c.read_pubkey()?;
        let _bump = c.read_u8()?;
        let mint_x = c.read_pubkey()?;
        let mint_y = c.read_pubkey()?;
        let concentration = c.read_u64()?;
        let big_k = c.read_u128()?;
        let target_x = c.read_u64()?;
        let _cumulative_volume = c.read_u64()?;
        let mult_x = c.read_u64()?;
        let mult_y = c.read_u64()?;
        let fee_millionth = c.read_u64()?;
        let rebate_percentage = c.read_u64()?;
        let protocol_fee_share_thousandth = c.read_u64()?;
        // Remaining fields (volume_record, volume_time_record, version, padding, sslp mints,
        // padding2) are not needed for quoting and are left unread.

        Ok(SSTradingPair {
            is_initialized,
            x_price_feed_id,
            y_price_feed_id,
            reserve_x,
            reserve_y,
            mint_x,
            mint_y,
            concentration,
            big_k,
            target_x,
            mult_x,
            mult_y,
            fee_millionth,
            rebate_percentage,
            protocol_fee_share_thousandth,
        })
    }

    /// Recompute `mult_x`/`mult_y` from fresh oracle prices and token decimals, matching the
    /// program's own `update_price`. Prices are Pyth values scaled to exponent -3 (i.e. ×1000).
    /// Uses saturating multiply so a bad feed can never panic the quoter.
    pub fn update_price(&mut self, price_x: u64, price_y: u64, x_decimals: u8, y_decimals: u8) {
        let (x_deci_mult, y_deci_mult) = match x_decimals.cmp(&y_decimals) {
            core::cmp::Ordering::Greater => {
                (1u64, 10u64.saturating_pow((x_decimals - y_decimals) as u32))
            }
            core::cmp::Ordering::Less => {
                (10u64.saturating_pow((y_decimals - x_decimals) as u32), 1u64)
            }
            core::cmp::Ordering::Equal => (1u64, 1u64),
        };
        self.mult_x = price_x.saturating_mul(x_deci_mult);
        self.mult_y = price_y.saturating_mul(y_deci_mult);
    }
}
