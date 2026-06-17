//! A single ground-truth observation of a venue's quote.

use propquote_core::types::Side;

/// One `(state, amount_in) -> amount_out` observation, with the venue state that was live at the
/// time of the fill. All amounts are raw token units; `mult_x`/`mult_y` are the oracle-derived
/// value scalers (price scaled, e.g. ×1000), matching the Obric convention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub current_x: u64,
    pub current_y: u64,
    pub mult_x: u64,
    pub mult_y: u64,
    /// Inventory target for X. For venues where this isn't directly observable, pass the value you
    /// believe it to be (often the mid of the reserves) — the fit absorbs small misspecification.
    pub target_x: u64,
    pub side: Side,
    pub amount_in: u64,
    pub amount_out: u64,
}
