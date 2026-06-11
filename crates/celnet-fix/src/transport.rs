//! Async FIX frame transport over a `tokio` byte stream.
//!
//! FIX has no length prefix — a frame is delimited by its `10=<cs>SOH` trailer.
//! [`FrameReader`] reads bytes from any [`AsyncRead`] into a growable buffer and
//! yields complete frames as they arrive, leaving any partial tail for the next
//! read. This is the only place the engine touches a socket; it lives on the
//! async edge and never on the pinned hot path.
//!
//! # Inbound message-size cap
//!
//! The accumulation buffer is hard-capped at [`MAX_FIX_MESSAGE_BYTES`].  A peer
//! that streams an unbounded byte run without a valid `10=NNN\x01` trailer
//! (malformed or adversarial) would otherwise exhaust memory on the acceptor —
//! a memory-exhaustion DoS (see Round-3 P1-fix issue dos/fix-frame-accumulation-
//! unbounded).  This mirrors the WS-edge design in
//! `crates/celnet-server/src/ws/limits.rs`: an explicit cap enforced at the
//! transport seam, surfaced as a typed `InvalidData` I/O error that the acceptor
//! loop propagates as a clean session teardown.
//!
//! ## Sizing rationale
//!
//! A FIX 4.4 application message is a flat `tag=value\x01` run.  The widest
//! legitimate shapes carried by this engine are:
//!
//! * `QuoteRequest(R)` — prologue + a dozen instrument/strategy fields +
//!   multi-leg repeating group (≈ 10 legs × ≈ 200 B each) = < 4 KiB.
//! * `ExecutionReport(8)` — similar breadth, ≤ 2 KiB.
//! * `MassQuote(i)` — up to ~100 quote entries × ≈ 100 B = ≈ 10 KiB in the most
//!   generous sizing; the FIX spec's own example fits in a single TCP segment.
//!
//! [`MAX_FIX_MESSAGE_BYTES`] = 64 KiB is therefore **> 6× the widest legitimate
//! frame** (with headroom for future field additions) while being too small for
//! any remotely plausible memory-exhaustion attack.  The figure matches the
//! conventional wisdom that "a FIX app message fits in a single Ethernet jumbo
//! frame" (9000-byte MTU) — 64 KiB is seven times that generous ceiling.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::framing::{SOH, parse_uint};

/// Hard cap on the inbound frame-accumulation buffer, in bytes (64 KiB).
///
/// Any peer whose byte stream causes the buffer to reach this size without a
/// complete `10=NNN\x01` trailer is rejected: [`FrameReader::next_frame`]
/// returns an [`std::io::ErrorKind::InvalidData`] error and the connection is
/// torn down.  Every legitimate FIX message produced by this engine is < 4 KiB;
/// this bound provides > 16× headroom.  See the module-level sizing rationale.
pub const MAX_FIX_MESSAGE_BYTES: usize = 64 * 1024;

/// Reads complete FIX frames from an [`AsyncRead`], buffering partial input.
#[derive(Debug)]
pub struct FrameReader<R> {
    inner: R,
    buf: Vec<u8>,
    /// Scan offset: bytes before this have already been searched for a trailer.
    scan: usize,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    /// Wrap a reader.
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            buf: Vec::with_capacity(4096),
            scan: 0,
        }
    }

    /// Read and return the next complete frame's bytes, or `None` on a clean
    /// EOF with no buffered partial frame.
    ///
    /// # Errors
    ///
    /// Returns `Err(InvalidData)` if the inbound byte stream accumulates more
    /// than [`MAX_FIX_MESSAGE_BYTES`] bytes without a valid `10=NNN\x01`
    /// trailer — the peer is streaming an unbounded or oversized frame and the
    /// connection must be torn down.
    ///
    /// Propagates any other I/O error from the underlying reader.
    pub async fn next_frame(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(end) = self.find_frame_end() {
                let frame = self.buf.drain(..end).collect::<Vec<u8>>();
                self.scan = 0;
                return Ok(Some(frame));
            }
            let mut chunk = [0u8; 4096];
            let n = self.inner.read(&mut chunk).await?;
            if n == 0 {
                if self.buf.is_empty() {
                    return Ok(None);
                }
                // Partial frame at EOF — treat as a truncated stream.
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "EOF mid-frame",
                ));
            }
            // Enforce the inbound message-size cap BEFORE extending the buffer.
            // If the new bytes would push us past the cap, refuse and tear down
            // the session: returning InvalidData causes the acceptor run-loop to
            // surface a clean I/O error, closing the TCP connection.
            if self.buf.len() + n > MAX_FIX_MESSAGE_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "inbound FIX frame exceeds the {MAX_FIX_MESSAGE_BYTES}-byte cap \
                         (buffer was {} bytes, read {n} more); connection torn down",
                        self.buf.len(),
                    ),
                ));
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// Locate the end offset (exclusive) of the first complete frame in the
    /// buffer by finding a `10=<3 digits>SOH` trailer. Returns the index one
    /// past the trailing `SOH`.
    fn find_frame_end(&mut self) -> Option<usize> {
        // The checksum field is "10=NNN\x01" — 7 bytes. Search for "10=" that
        // is at a field boundary (preceded by SOH) and followed by 3 digits and
        // an SOH.
        let buf = &self.buf;
        let mut i = self.scan;
        while i + 7 <= buf.len() {
            // A field boundary: position 0 or preceded by SOH.
            let at_boundary = i == 0 || buf[i - 1] == SOH;
            if at_boundary && &buf[i..i + 3] == b"10=" {
                let digits = &buf[i + 3..i + 6];
                if buf[i + 6] == SOH && parse_uint(digits).is_some() && digits.len() == 3 {
                    return Some(i + 7);
                }
            }
            i += 1;
        }
        // Remember how far we scanned (keep a little overlap for "10=" split
        // across reads).
        self.scan = buf.len().saturating_sub(6);
        None
    }
}

/// Write a complete frame to an [`AsyncWrite`], flushing it.
///
/// # Errors
/// Propagates I/O errors from the underlying writer.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, frame: &[u8]) -> std::io::Result<()> {
    w.write_all(frame).await?;
    w.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::FrameEncoder;

    fn hb() -> Vec<u8> {
        let mut e = FrameEncoder::new();
        e.push(35, b"0");
        e.push(49, b"A");
        e.push(56, b"B");
        e.finish()
    }

    #[tokio::test]
    async fn reads_concatenated_frames() {
        let a = hb();
        let b = hb();
        let mut stream = a.clone();
        stream.extend_from_slice(&b);
        let mut reader = FrameReader::new(std::io::Cursor::new(stream));
        let f1 = reader.next_frame().await.unwrap().unwrap();
        let f2 = reader.next_frame().await.unwrap().unwrap();
        assert_eq!(f1, a);
        assert_eq!(f2, b);
        assert!(reader.next_frame().await.unwrap().is_none());
    }

    /// A frame that exactly fills the cap is read and returned normally — the
    /// cap check is `buf.len() + n > MAX_FIX_MESSAGE_BYTES`; a frame whose total
    /// size equals the cap arrives in the final chunk without tripping the guard.
    ///
    /// We build a real, valid FIX heartbeat padded with a large `Text(58)` field
    /// so the finished frame is exactly `MAX_FIX_MESSAGE_BYTES` bytes long.
    ///
    /// # Size arithmetic (verified at runtime by the assertion below)
    ///
    /// A FIX frame layout: `8=FIX.4.4\x01` (10 B) + `9=<BL>\x01` + body +
    /// `10=NNN\x01` (7 B).  For total T = 65536:
    ///   total = 10 + (3 + digits(BL)) + BL + 7 = 20 + digits(BL) + BL
    ///   → BL = 65511 (5-digit, 10 000–99 999; 20+5+65511 = 65536 ✓).
    /// Minimum body (35=, 49=, 56= = 15 B) + `58=<P>\x01` (4+P B):
    ///   15 + 4 + P = 65511 → P = 65492.
    #[tokio::test]
    async fn frame_at_the_cap_is_accepted() {
        // Derived above; the assertion below catches any arithmetic drift.
        const TEXT_PADDING: usize = 65492;

        let padding = vec![b'X'; TEXT_PADDING];
        let mut enc = FrameEncoder::new();
        enc.push(35, b"0"); // MsgType = Heartbeat
        enc.push(49, b"A"); // SenderCompID
        enc.push(56, b"B"); // TargetCompID
        enc.push(58, &padding); // Text: the padding field
        let frame = enc.finish();
        assert_eq!(
            frame.len(),
            MAX_FIX_MESSAGE_BYTES,
            "frame must be exactly the cap ({MAX_FIX_MESSAGE_BYTES} B); \
             check the TEXT_PADDING constant if this fails"
        );

        let mut reader = FrameReader::new(std::io::Cursor::new(frame.clone()));
        let got = reader
            .next_frame()
            .await
            .expect("a frame exactly at the cap is accepted — no InvalidData")
            .expect("a complete frame was returned");
        assert_eq!(got, frame, "the returned bytes match the input");
    }

    /// A frame one byte over the cap is refused with a typed `InvalidData`
    /// error — the accumulation buffer is never allowed to exceed the cap.
    #[tokio::test]
    async fn frame_over_the_cap_is_refused_with_typed_error() {
        // One byte more than the cap: build a padding blob with no valid FIX
        // trailer so the reader keeps accumulating until it hits the limit.
        let oversize: Vec<u8> = vec![b'X'; MAX_FIX_MESSAGE_BYTES + 1];

        let mut reader = FrameReader::new(std::io::Cursor::new(oversize));
        let err = reader
            .next_frame()
            .await
            .expect_err("an oversize stream must be refused");
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::InvalidData,
            "the refusal must be typed InvalidData, not a different kind: {err}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains(&MAX_FIX_MESSAGE_BYTES.to_string()),
            "the error must name the cap so the caller can log it: {msg}"
        );
    }
}
