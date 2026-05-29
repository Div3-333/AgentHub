//! ZSTD codec for `pty_debug_log` blobs (Part 14 / Part 19 #9).

use crate::error::{AgentHubError, Result};

const ZSTD_LEVEL: i32 = 3;

/// ZSTD-compress raw PTY bytes for storage.
pub fn compress_pty_bytes(raw: &[u8]) -> Result<Vec<u8>> {
    zstd::encode_all(raw, ZSTD_LEVEL).map_err(|e| AgentHubError::Pty(format!("zstd encode: {e}")))
}

/// Decompress a stored PTY debug blob (tolerates legacy uncompressed rows).
pub fn decompress_pty_bytes(stored: &[u8]) -> Result<Vec<u8>> {
    match zstd::decode_all(stored) {
        Ok(bytes) => Ok(bytes),
        Err(_) => Ok(stored.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_roundtrip() {
        let raw = b"\x1b[31mhello\x1b[0m";
        let compressed = compress_pty_bytes(raw).expect("compress");
        assert_ne!(compressed.as_slice(), raw.as_slice());
        let restored = decompress_pty_bytes(&compressed).expect("decompress");
        assert_eq!(restored, raw);
    }
}
