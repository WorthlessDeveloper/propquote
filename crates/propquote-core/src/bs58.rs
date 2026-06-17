//! Minimal base58 (Bitcoin alphabet) decode/encode for Solana pubkeys — no external crate.

use crate::types::{Pubkey, QuoteError};

const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Decode a base58 string into raw bytes (big-endian).
pub fn decode(s: &str) -> Result<Vec<u8>, QuoteError> {
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
    for c in s.bytes() {
        let mut carry = ALPHABET
            .iter()
            .position(|&a| a == c)
            .ok_or(QuoteError::InvalidData)? as u32;
        for b in bytes.iter_mut() {
            carry += (*b as u32) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    // Leading '1's encode leading zero bytes.
    for c in s.bytes() {
        if c == b'1' {
            bytes.push(0);
        } else {
            break;
        }
    }
    bytes.reverse();
    Ok(bytes)
}

/// Decode a base58 string expected to be exactly a 32-byte pubkey.
pub fn decode_32(s: &str) -> Result<Pubkey, QuoteError> {
    let bytes = decode(s)?;
    if bytes.len() > 32 {
        return Err(QuoteError::InvalidData);
    }
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(&bytes);
    Ok(out)
}

/// Encode raw bytes as a base58 string.
pub fn encode(input: &[u8]) -> String {
    let mut digits: Vec<u8> = Vec::with_capacity(input.len() * 2);
    for &byte in input {
        let mut carry = byte as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut out = String::with_capacity(digits.len() + input.len());
    for &byte in input {
        if byte == 0 {
            out.push('1');
        } else {
            break;
        }
    }
    for &d in digits.iter().rev() {
        out.push(ALPHABET[d as usize] as char);
    }
    if out.is_empty() {
        out.push('1');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_known_pubkey() {
        // Wrapped SOL mint.
        let s = "So11111111111111111111111111111111111111112";
        let k = decode_32(s).unwrap();
        assert_eq!(encode(&k), s);
    }

    #[test]
    fn roundtrips_obric_program_id() {
        let s = "obriQD1zbpyLz95G5n7nJe6a4DPjpFwa5XYPoNm113y";
        let k = decode_32(s).unwrap();
        assert_eq!(encode(&k), s);
    }

    #[test]
    fn rejects_bad_char() {
        assert_eq!(decode("0OIl"), Err(QuoteError::InvalidData));
    }
}
