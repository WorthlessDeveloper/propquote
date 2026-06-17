//! A tiny bounds-checked little-endian cursor for parsing raw account bytes.
//!
//! Borsh/Anchor layouts are just packed little-endian fields. Rather than depend on `borsh`, we
//! read fields explicitly — which also documents each on-chain struct precisely at the call site.

use crate::types::{Pubkey, QuoteError};

pub struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Cursor { data, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], QuoteError> {
        let end = self.pos.checked_add(n).ok_or(QuoteError::ShortBuffer)?;
        if end > self.data.len() {
            return Err(QuoteError::ShortBuffer);
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub fn skip(&mut self, n: usize) -> Result<(), QuoteError> {
        self.take(n).map(|_| ())
    }

    pub fn read_u8(&mut self) -> Result<u8, QuoteError> {
        Ok(self.take(1)?[0])
    }

    pub fn read_bool(&mut self) -> Result<bool, QuoteError> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(QuoteError::InvalidData),
        }
    }

    pub fn read_u64(&mut self) -> Result<u64, QuoteError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(b.try_into().map_err(|_| QuoteError::ShortBuffer)?))
    }

    pub fn read_i64(&mut self) -> Result<i64, QuoteError> {
        let b = self.take(8)?;
        Ok(i64::from_le_bytes(b.try_into().map_err(|_| QuoteError::ShortBuffer)?))
    }

    pub fn read_u128(&mut self) -> Result<u128, QuoteError> {
        let b = self.take(16)?;
        Ok(u128::from_le_bytes(b.try_into().map_err(|_| QuoteError::ShortBuffer)?))
    }

    pub fn read_pubkey(&mut self) -> Result<Pubkey, QuoteError> {
        let b = self.take(32)?;
        b.try_into().map_err(|_| QuoteError::ShortBuffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_fields_in_order() {
        let mut buf = Vec::new();
        buf.push(1u8); // bool true
        buf.extend_from_slice(&7u64.to_le_bytes());
        buf.extend_from_slice(&[9u8; 32]); // pubkey
        buf.extend_from_slice(&42u128.to_le_bytes());

        let mut c = Cursor::new(&buf);
        assert_eq!(c.read_bool().unwrap(), true);
        assert_eq!(c.read_u64().unwrap(), 7);
        assert_eq!(c.read_pubkey().unwrap(), [9u8; 32]);
        assert_eq!(c.read_u128().unwrap(), 42);
        assert_eq!(c.remaining(), 0);
    }

    #[test]
    fn short_buffer_errors() {
        let buf = [0u8; 4];
        let mut c = Cursor::new(&buf);
        assert_eq!(c.read_u64(), Err(QuoteError::ShortBuffer));
    }

    #[test]
    fn bad_bool_errors() {
        let buf = [2u8];
        let mut c = Cursor::new(&buf);
        assert_eq!(c.read_bool(), Err(QuoteError::InvalidData));
    }
}
