//! Async FIX frame transport over a `tokio` byte stream.
//!
//! FIX has no length prefix — a frame is delimited by its `10=<cs>SOH` trailer.
//! [`FrameReader`] reads bytes from any [`AsyncRead`] into a growable buffer and
//! yields complete frames as they arrive, leaving any partial tail for the next
//! read. This is the only place the engine touches a socket; it lives on the
//! async edge and never on the pinned hot path.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::framing::{SOH, parse_uint};

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
    /// Propagates I/O errors from the underlying reader.
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
}
