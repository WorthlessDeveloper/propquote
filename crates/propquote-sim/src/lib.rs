//! `propquote-sim` — the ground-truth quote oracle.
//!
//! For the obfuscated venues we can't (yet) write the closed form, the only source of truth is the
//! venue's own bytecode. This crate loads a venue's real `.so` into [LiteSVM](https://crates.io/crates/litesvm)
//! together with the live account state, executes a swap against it, and returns the realized
//! output (the destination-token balance delta). Feed those outputs to
//! [`propquote_replay::fit_obric_form`] to recover a fast closed form.
//!
//! ## What you provide at runtime
//! - the venue program `.so` (re-dump from mainnet with `solana program dump <PROGRAM_ID>`; we do
//!   not redistribute third-party binaries), and
//! - the live accounts the swap reads (pool/market, vaults, oracle), fetched from an RPC.
//!
//! Build the per-venue instruction with [`propquote_replay::venues`] (selector + calldata) and the
//! documented account ordering, then call [`GroundTruthSvm::simulate_swap`].
//!
//! ## Verification note
//! This crate is excluded from the workspace `default-members` and is compiled in CI on Linux.
//! It is not part of the dependency-light core; a quote from `propquote-obric` needs none of this.

pub mod account_json;

use litesvm::LiteSVM;
use solana_compute_budget::compute_budget::ComputeBudget;
use solana_instruction::{AccountMeta, Instruction};
use solana_sdk::{
    account::Account, program_pack::Pack, pubkey::Pubkey, rent::Rent, signature::Keypair,
    signer::Signer, transaction::Transaction,
};
use spl_associated_token_account::get_associated_token_address;
use std::path::Path;

#[derive(thiserror::Error, Debug)]
pub enum SimError {
    #[error("failed to load program {0}: {1}")]
    LoadProgram(Pubkey, String),
    #[error("failed to set account {0}: {1}")]
    SetAccount(Pubkey, String),
    #[error("transaction failed: {0}")]
    Transaction(String),
    #[error("missing token account {0}")]
    MissingTokenAccount(Pubkey),
}

pub type Result<T> = std::result::Result<T, SimError>;

/// An in-process Solana VM preloaded with a funded wallet, used to run real venue programs.
pub struct GroundTruthSvm {
    pub svm: LiteSVM,
    pub wallet: Keypair,
}

impl Default for GroundTruthSvm {
    fn default() -> Self {
        Self::new()
    }
}

impl GroundTruthSvm {
    const AIRDROP_LAMPORTS: u64 = 100_000_000;
    const COMPUTE_UNIT_LIMIT: u64 = 20_000_000;

    /// Create a VM with default programs/sysvars, signature verification on, and a funded wallet.
    pub fn new() -> Self {
        let mut budget = ComputeBudget::new_with_defaults(true);
        budget.compute_unit_limit = Self::COMPUTE_UNIT_LIMIT;

        let svm = LiteSVM::new()
            .with_default_programs()
            .with_sysvars()
            .with_sigverify(true)
            .with_compute_budget(budget);

        let wallet = Keypair::new();
        let mut this = GroundTruthSvm { svm, wallet };
        this.svm
            .airdrop(&this.wallet.pubkey(), Self::AIRDROP_LAMPORTS)
            .expect("airdrop to fresh wallet should not fail");
        this
    }

    pub fn wallet_pubkey(&self) -> Pubkey {
        self.wallet.pubkey()
    }

    /// Load a program `.so` at a given program id.
    pub fn load_program(&mut self, program_id: Pubkey, path: impl AsRef<Path>) -> Result<()> {
        self.svm
            .add_program_from_file(program_id, path)
            .map_err(|e| SimError::LoadProgram(program_id, format!("{e:?}")))
    }

    /// Inject a raw account (e.g. a pool/market/vault/oracle account fetched from mainnet).
    pub fn set_account(&mut self, pubkey: Pubkey, account: Account) -> Result<()> {
        self.svm
            .set_account(pubkey, account)
            .map_err(|e| SimError::SetAccount(pubkey, format!("{e:?}")))
    }

    pub fn warp_to_slot(&mut self, slot: u64) {
        self.svm.warp_to_slot(slot);
    }

    /// Create an initialized SPL mint with the given decimals.
    pub fn create_mint(&mut self, mint: Pubkey, decimals: u8) -> Result<()> {
        let state = spl_token::state::Mint {
            mint_authority: solana_sdk::program_option::COption::None,
            supply: u64::MAX,
            decimals,
            is_initialized: true,
            freeze_authority: solana_sdk::program_option::COption::None,
        };
        let mut data = vec![0u8; spl_token::state::Mint::LEN];
        spl_token::state::Mint::pack(state, &mut data).expect("mint pack");
        self.set_account(mint, token_owned_account(data))
    }

    /// Create the wallet's associated token account for `mint`, pre-funded with `amount`.
    /// Returns the ATA address (use it as the swap's source/destination token account).
    pub fn create_wallet_ata(&mut self, mint: Pubkey, amount: u64) -> Result<Pubkey> {
        let owner = self.wallet_pubkey();
        let ata = get_associated_token_address(&owner, &mint);
        let state = spl_token::state::Account {
            mint,
            owner,
            amount,
            state: spl_token::state::AccountState::Initialized,
            ..Default::default()
        };
        let mut data = vec![0u8; spl_token::state::Account::LEN];
        state.pack_into_slice(&mut data);
        self.set_account(ata, token_owned_account(data))?;
        Ok(ata)
    }

    /// Current SPL token balance of a token account (0 if absent/unreadable).
    pub fn token_balance(&self, token_account: Pubkey) -> u64 {
        self.svm
            .get_account(&token_account)
            .and_then(|a| spl_token::state::Account::unpack(&a.data).ok())
            .map(|a| a.amount)
            .unwrap_or(0)
    }

    /// Execute one swap instruction against a loaded venue program and return the realized output
    /// as the destination token account's balance delta — the ground-truth `amount_out`.
    ///
    /// Build `accounts` and `data` with [`propquote_replay::venues`] for the target venue, mapping
    /// the documented account roles to the live pubkeys you loaded via [`Self::set_account`].
    pub fn simulate_swap(
        &mut self,
        program_id: Pubkey,
        accounts: Vec<AccountMeta>,
        data: Vec<u8>,
        dst_token_account: Pubkey,
    ) -> Result<u64> {
        let before = self.token_balance(dst_token_account);

        let ix = Instruction {
            program_id,
            accounts,
            data,
        };
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.wallet_pubkey()),
            &[&self.wallet],
            self.svm.latest_blockhash(),
        );
        self.svm
            .send_transaction(tx)
            .map_err(|e| SimError::Transaction(format!("{e:?}")))?;

        let after = self.token_balance(dst_token_account);
        Ok(after.saturating_sub(before))
    }
}

/// Build a rent-exempt, SPL-token-owned account from packed data.
fn token_owned_account(data: Vec<u8>) -> Account {
    Account {
        lamports: Rent::default().minimum_balance(data.len()),
        data,
        owner: spl_token::id(),
        executable: false,
        rent_epoch: u64::MAX,
    }
}
