//! Reverse-engineered swap-instruction calldata for the obfuscated prop AMMs.
//!
//! The hard, venue-specific part of driving a real program is its instruction *data* (selector +
//! argument encoding) and account *ordering*. Those are reconstructed here, byte-for-byte, and
//! unit-tested. `propquote-sim` uses these to build transactions against each venue's real `.so`.
//!
//! Account ordering is given as documentation (the actual pubkeys come from chain at runtime).
//! Layouts and selectors were recovered from the public on-chain programs.

/// SolFi V2 — `SV2EYYJyRz2YhfXwXnhNAevDEui5Q6yrfyo13WtupPF`.
pub mod solfi_v2 {
    pub const PROGRAM_ID: &str = "SV2EYYJyRz2YhfXwXnhNAevDEui5Q6yrfyo13WtupPF";
    pub const SWAP_SELECTOR: u8 = 0x07;

    /// Swap calldata: `selector ++ borsh{ amount_in: u64, min_amount_out: u64, direction: u8 }`.
    /// `direction`: `0` = base→quote, `1` = quote→base. Total length 18 bytes.
    pub fn swap_data(amount_in: u64, min_amount_out: u64, direction: u8) -> Vec<u8> {
        let mut d = Vec::with_capacity(18);
        d.push(SWAP_SELECTOR);
        d.extend_from_slice(&amount_in.to_le_bytes());
        d.extend_from_slice(&min_amount_out.to_le_bytes());
        d.push(direction);
        d
    }

    /// Account order expected by the swap instruction (`*` = signer).
    pub const ACCOUNTS: &[&str] = &[
        "swap_authority*",
        "market",
        "oracle",
        "global_config",
        "base_vault",
        "quote_vault",
        "user_base_ta",
        "user_quote_ta",
        "base_mint",
        "quote_mint",
        "base_token_program",
        "quote_token_program",
        "instructions_sysvar",
    ];
}

/// ZeroFi — `ZERor4xhbUycZ6gb9ntrhqscUcZmAbQDjEAtCf4hbZY`.
pub mod zerofi {
    pub const PROGRAM_ID: &str = "ZERor4xhbUycZ6gb9ntrhqscUcZmAbQDjEAtCf4hbZY";
    /// ZeroFi has no separate selector; the discriminator is the first byte of the args struct.
    pub const SWAP_DISCRIMINATOR: u8 = 0x06;

    /// Swap calldata: `borsh{ discriminator: u8 = 6, amount_in: u64, desired_output: u64 }`.
    /// Total length 17 bytes.
    pub fn swap_data(amount_in: u64, desired_output: u64) -> Vec<u8> {
        let mut d = Vec::with_capacity(17);
        d.push(SWAP_DISCRIMINATOR);
        d.extend_from_slice(&amount_in.to_le_bytes());
        d.extend_from_slice(&desired_output.to_le_bytes());
        d
    }

    /// Account order expected by the swap instruction (`*` = signer). `*_in`/`*_out` are resolved
    /// from the swap direction (base vs quote).
    pub const ACCOUNTS: &[&str] = &[
        "pair",
        "vault_info_in",
        "vault_in",
        "vault_info_out",
        "vault_out",
        "user_src_ta",
        "user_dst_ta",
        "swap_authority*",
        "token_program",
        "instructions_sysvar",
    ];
}

/// Tessera (TesseraV) — `TessVdML9pBGgG9yGks7o4HewRaXVAMuoVj4x83GLQH`.
pub mod tessera {
    pub const PROGRAM_ID: &str = "TessVdML9pBGgG9yGks7o4HewRaXVAMuoVj4x83GLQH";
    pub const SWAP_SELECTOR: u8 = 0x10;

    /// Swap calldata: `selector ++ borsh{ side: u8, amount_in: u64, min_amount_out: u64 }`.
    /// `side`: `1` = base→quote, `0` = quote→base. Total length 18 bytes.
    pub fn swap_data(side: u8, amount_in: u64, min_amount_out: u64) -> Vec<u8> {
        let mut d = Vec::with_capacity(18);
        d.push(SWAP_SELECTOR);
        d.push(side);
        d.extend_from_slice(&amount_in.to_le_bytes());
        d.extend_from_slice(&min_amount_out.to_le_bytes());
        d
    }

    /// Account order expected by the swap instruction (`*` = signer).
    pub const ACCOUNTS: &[&str] = &[
        "global_state",
        "pool_state",
        "swap_authority*",
        "base_vault",
        "quote_vault",
        "base_ta",
        "quote_ta",
        "base_mint",
        "quote_mint",
        "base_token_program",
        "quote_token_program",
        "instructions_sysvar",
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solfi_v2_calldata() {
        let d = solfi_v2::swap_data(1_000_000, 990_000, 1);
        assert_eq!(d.len(), 18);
        assert_eq!(d[0], 0x07);
        assert_eq!(&d[1..9], &1_000_000u64.to_le_bytes());
        assert_eq!(&d[9..17], &990_000u64.to_le_bytes());
        assert_eq!(d[17], 1);
    }

    #[test]
    fn zerofi_calldata() {
        let d = zerofi::swap_data(1_000_000, 0);
        assert_eq!(d.len(), 17);
        assert_eq!(d[0], 0x06);
        assert_eq!(&d[1..9], &1_000_000u64.to_le_bytes());
        assert_eq!(&d[9..17], &0u64.to_le_bytes());
    }

    #[test]
    fn tessera_calldata() {
        let d = tessera::swap_data(1, 1_000_000, 0);
        assert_eq!(d.len(), 18);
        assert_eq!(d[0], 0x10);
        assert_eq!(d[1], 1);
        assert_eq!(&d[2..10], &1_000_000u64.to_le_bytes());
    }
}
