//! Parse the JSON emitted by `solana account <PUBKEY> --output json` into a `solana_sdk::Account`.
//!
//! Keeping account *fetching* in the CI shell (via the `solana` CLI) and only *parsing* here means
//! the sim crate needs no RPC client — far less surface to get wrong.

use base64::Engine;
use solana_sdk::account::Account;
use std::path::Path;

/// Read a `solana account --output json` file and reconstruct the on-chain account.
pub fn read_account_file(path: impl AsRef<Path>) -> Result<Account, String> {
    let raw = std::fs::read_to_string(path.as_ref())
        .map_err(|e| format!("read {:?}: {e}", path.as_ref()))?;
    let v: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("parse {:?}: {e}", path.as_ref()))?;
    let acc = v.get("account").unwrap_or(&v);

    let lamports = acc.get("lamports").and_then(|x| x.as_u64()).unwrap_or(0);
    let owner_str = acc
        .get("owner")
        .and_then(|x| x.as_str())
        .ok_or("missing owner")?;
    let owner = owner_str
        .parse::<solana_sdk::pubkey::Pubkey>()
        .map_err(|e| format!("bad owner {owner_str}: {e}"))?;
    let executable = acc
        .get("executable")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let rent_epoch = acc
        .get("rentEpoch")
        .and_then(|x| x.as_u64())
        .unwrap_or(u64::MAX);

    // `data` is `[ "<base64>", "base64" ]`.
    let b64 = acc
        .get("data")
        .and_then(|d| d.get(0))
        .and_then(|x| x.as_str())
        .ok_or("missing base64 data")?;
    let data = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("base64 decode: {e}"))?;

    Ok(Account {
        lamports,
        data,
        owner,
        executable,
        rent_epoch,
    })
}
