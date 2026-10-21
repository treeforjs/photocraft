//! Matching save/load budgets, checked before replacing any saved document.
use crate::{FormatError, Result};
use std::io::{self, Write};

pub(crate) fn account(total: &mut u64, bytes: usize, limit: u64) -> Result<()> {
    let next = u64::try_from(bytes).ok().and_then(|bytes| total.checked_add(bytes));
    match next {
        Some(next) if next <= limit => {
            *total = next;
            Ok(())
        }
        _ => Err(FormatError::LimitExceeded(format!("bundle expands beyond {limit} bytes"))),
    }
}

pub(crate) struct ManifestBuffer {
    pub(crate) bytes: Vec<u8>,
    limit: usize,
    pub(crate) exceeded: bool,
}
impl ManifestBuffer {
    pub(crate) fn new(limit: usize) -> Self {
        Self { bytes: Vec::new(), limit, exceeded: false }
    }
}
impl Write for ManifestBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().checked_add(bytes.len()).is_none_or(|size| size > self.limit) {
            self.exceeded = true;
            return Err(io::Error::new(io::ErrorKind::InvalidData, "manifest exceeds its reopen limit"));
        }
        self.bytes.try_reserve_exact(bytes.len()).map_err(io::Error::other)?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expansion_boundary_and_overflow_do_not_change_the_counter() {
        let mut total = 3;
        account(&mut total, 5, 8).unwrap();
        assert_eq!(total, 8);
        assert!(account(&mut total, 1, 8).is_err());
        assert_eq!(total, 8);
        total = u64::MAX;
        assert!(account(&mut total, 1, u64::MAX).is_err());
        assert_eq!(total, u64::MAX);
    }
    #[test]
    fn manifest_buffer_refuses_before_appending_beyond_the_limit() {
        let mut out = ManifestBuffer::new(8);
        out.write_all(b"12345678").unwrap();
        assert!(out.write_all(b"9").is_err());
        assert!(out.exceeded);
        assert_eq!(out.bytes, b"12345678");
        assert!(ManifestBuffer::new(0).write_all(b"x").is_err());
    }
}
