//! A tiny, self-contained CRC-32 (IEEE 802.3 / zlib polynomial `0xEDB88320`).
//!
//! The journal needs a per-record integrity check to detect a flipped byte or a
//! torn tail. We deliberately keep this **dependency-free**: a table-driven
//! reflected CRC-32 is a few lines, deterministic, and adds no supply-chain
//! surface (guardrail: OSS-only deps are vetted, and "no new dep" is cheaper
//! still). This is *not* a security primitive — it guards against accidental
//! corruption (partial writes, bit rot), which is exactly the journal's failure
//! model. The control-plane MAC/identity story lives elsewhere (`celnet-server`).
//!
//! The standard reflected algorithm with the IEEE polynomial is the same CRC-32
//! used by zlib/gzip/PNG, so the framing is interoperable with common tooling
//! for forensic inspection.

/// Reflected IEEE CRC-32 lookup table, built once at first use.
const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            // Reflected update: shift toward LSB, conditionally xor the polynomial.
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// The CRC table, computed at compile time (deterministic, no runtime init race).
static TABLE: [u32; 256] = build_table();

/// Compute the IEEE CRC-32 of `bytes`.
///
/// Uses the standard pre/post conditioning (`0xFFFF_FFFF` init, final xor with
/// `0xFFFF_FFFF`), matching zlib's `crc32`. Pure and allocation-free.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        let idx = ((crc ^ u32::from(b)) & 0xFF) as usize;
        crc = (crc >> 8) ^ TABLE[idx];
    }
    crc ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::crc32;

    #[test]
    fn known_vectors() {
        // Canonical zlib/IEEE test vectors.
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
    }

    #[test]
    fn single_bit_flip_changes_crc() {
        let a = crc32(b"celnet-journal");
        let b = crc32(b"celnet-journal"); // identical input ⇒ identical crc
        assert_eq!(a, b);
        let c = crc32(b"celnet-journbl"); // one byte changed
        assert_ne!(a, c);
    }
}
