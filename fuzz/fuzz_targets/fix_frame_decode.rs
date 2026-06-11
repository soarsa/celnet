//! Fuzz target: arbitrary bytes -> the `celnet-fix` untrusted FIX byte surface.
//!
//! `celnet-fix` is the ONLY engine component that parses bytes handed to us by
//! an EXTERNAL counterparty (LP / client FIX sessions over TCP) — the canonical
//! verification-contract clause-(f) attack surface. Two seams consume those
//! bytes, and both are driven here with no product logic re-implemented:
//!
//!   * `transport::FrameReader` — the stream DELIMITER: FIX has no length
//!     prefix, so raw socket bytes are scanned for the `10=<3 digits><SOH>`
//!     trailer and carved into frame candidates, buffering partial tails
//!     across reads (the cross-read scan-overlap rewind lives here).
//!   * `framing::FrameCursor::parse` — the frame VALIDATOR/parser: prologue
//!     (`8=`/`9=`), `BodyLength`, `CheckSum`, `MsgType`, then zero-copy field
//!     access.
//!
//! Contract under any input bytes (the house decode-target contract):
//!   * neither seam ever panics (no slice overrun, no `unwrap` on an
//!     attacker-controlled length or offset), and
//!   * `parse` returns `Ok(FrameCursor)` or a typed `FrameError` — never a
//!     partial/garbage cursor, never UB; the delimiter terminates, yields only
//!     trailer-terminated frames, and never invents bytes (bounded by the
//!     bytes actually read).
//!
//! Three phases per input:
//!   1. RAW — the bytes go straight into `FrameCursor::parse`; on `Ok` the
//!      body fields are walked (bounded, terminating; every value a borrowed
//!      subslice of the input) and the parsed `(tag, value)` sequence is
//!      re-encoded with the real `FrameEncoder` and re-parsed — the sequence
//!      and `MsgType` must round-trip. (Byte-identity is NOT asserted: a
//!      leading-zero tag like `035=` parses to 35 and re-encodes canonically.)
//!   2. STRUCTURED — `arbitrary` folds the same bytes into an in-domain field
//!      list laid out by the REAL encoder. The result must parse `Ok`
//!      (encoder/decoder agreement) and round-trip field-for-field; then a
//!      single-byte XOR and a strict truncation must each be REJECTED (any
//!      one-byte change shifts the mod-256 checksum away from the declared
//!      trailer value or breaks the structure first; any strict prefix loses
//!      the exactly-trailing checksum field). This phase keeps the
//!      coverage-guided search permanently past the checksum wall that raw
//!      random bytes almost never cross.
//!   3. STREAM — the bytes are fed to `FrameReader` in adversarial chunk
//!      sizes (splitting the trailer across reads), asserting the delimiter
//!      terminates, never invents bytes, ends every yielded frame with a
//!      well-formed trailer, and that delimit→parse upholds the no-panic /
//!      typed-`Err` contract on whatever was carved out.
//!
//! Stable-gate mirror (blocks merges even when this nightly lane does not
//! run): `crates/celnet-fix/tests/codec_roundtrip.rs` (proptest —
//! arbitrary-bytes no-panic, single-byte-corruption rejection, and the
//! malformed-frame battery).
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run fix_frame_decode -- -max_total_time=120

#![no_main]

use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};

use arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;
use tokio::io::{AsyncRead, ReadBuf};

use celnet_fix::framing::{FrameCursor, FrameEncoder, SOH, TAG_CHECK_SUM, TAG_MSG_TYPE};
use celnet_fix::transport::FrameReader;

fuzz_target!(|data: &[u8]| {
    raw_parse(data);
    structured_roundtrip(data);
    delimit_stream(data);
});

/// Phase 1 — RAW: adversarial bytes straight into the frame parser.
fn raw_parse(data: &[u8]) {
    let Ok(frame) = FrameCursor::parse(data) else {
        // A typed `FrameError` — an honest rejection; nothing more to check.
        return;
    };
    // Bounded, terminating field walk: every value is a borrowed subslice of
    // the input (zero-copy, so bounded by construction), and a well-formed
    // field occupies at least 3 body bytes (`t=<SOH>`), bounding the count.
    let fields: Vec<(u32, &[u8])> = frame.fields().map(|f| (f.tag, f.value)).collect();
    for (_, value) in &fields {
        assert!(
            value.len() <= data.len(),
            "field value longer than the input it was carved from"
        );
    }
    assert!(
        fields.len() <= data.len() / 3,
        "more body fields than the input can hold"
    );
    // Soundness: re-encode the parsed sequence with the real encoder and
    // re-parse — the `(tag, value)` sequence and `MsgType` must round-trip.
    let mut enc = FrameEncoder::new();
    for (tag, value) in &fields {
        enc.push(*tag, value);
    }
    let re = enc.finish();
    let back = FrameCursor::parse(&re).expect("re-encoded accepted frame must parse");
    assert_eq!(back.msg_type(), frame.msg_type(), "MsgType round-trip");
    let back_fields: Vec<(u32, &[u8])> = back.fields().map(|f| (f.tag, f.value)).collect();
    assert_eq!(fields, back_fields, "(tag, value) sequence round-trip");
}

/// The structured phase-2 input: an in-domain frame plus corruption knobs.
struct FrameSpec {
    msg_type: Vec<u8>,
    fields: Vec<(u32, Vec<u8>)>,
    flip_pos: usize,
    flip_mask: u8,
    cut: usize,
}

/// Fold fuzzer bytes into an in-domain `FrameSpec`.
fn frame_spec(u: &mut Unstructured<'_>) -> arbitrary::Result<FrameSpec> {
    let msg_type = in_domain_value(u, 8)?;
    let n: usize = u.int_in_range(0..=12)?;
    let mut fields = Vec::with_capacity(n);
    for _ in 0..n {
        let tag: u32 = u.arbitrary()?;
        // Tag 10 is the trailer: the encoder's documented domain excludes it
        // from the body (the parser must treat the first `10=` field boundary
        // as the checksum), so map it away rather than waste the exec.
        let tag = if tag == TAG_CHECK_SUM {
            TAG_CHECK_SUM + 1
        } else {
            tag
        };
        fields.push((tag, in_domain_value(u, 32)?));
    }
    Ok(FrameSpec {
        msg_type,
        fields,
        flip_pos: usize::from(u.arbitrary::<u16>()?),
        // XOR with a zero mask would be the identity — force at least one bit.
        flip_mask: u.arbitrary::<u8>()?.max(1),
        cut: usize::from(u.arbitrary::<u16>()?),
    })
}

/// An arbitrary field value inside the encoder's documented domain: raw bytes
/// with the `SOH` delimiter mapped away (a value containing `SOH` would split
/// the field on the wire — the message builders never emit one).
fn in_domain_value(u: &mut Unstructured<'_>, max: usize) -> arbitrary::Result<Vec<u8>> {
    let len: usize = u.int_in_range(0..=max)?;
    let mut v = vec![0u8; len];
    for b in &mut v {
        let raw: u8 = u.arbitrary()?;
        *b = if raw == SOH { 0x02 } else { raw };
    }
    Ok(v)
}

/// Phase 2 — STRUCTURED: encoder/decoder agreement + detectable corruption.
fn structured_roundtrip(data: &[u8]) {
    let mut u = Unstructured::new(data);
    let Ok(spec) = frame_spec(&mut u) else { return };
    let mut enc = FrameEncoder::new();
    enc.push(TAG_MSG_TYPE, &spec.msg_type);
    for (tag, value) in &spec.fields {
        enc.push(*tag, value);
    }
    let raw = enc.finish();

    // Agreement: the real encoder's output over in-domain fields is always
    // accepted by the real parser — a permanent foothold past the checksum.
    let frame = FrameCursor::parse(&raw)
        .unwrap_or_else(|e| panic!("encoder output rejected by parser: {e:?}"));
    assert_eq!(
        frame.msg_type(),
        spec.msg_type.as_slice(),
        "MsgType round-trip"
    );
    let got: Vec<(u32, &[u8])> = frame.fields().map(|f| (f.tag, f.value)).collect();
    assert_eq!(got.len(), spec.fields.len() + 1, "field count round-trip");
    assert_eq!(got[0], (TAG_MSG_TYPE, spec.msg_type.as_slice()));
    for (g, w) in got[1..].iter().zip(&spec.fields) {
        assert_eq!(*g, (w.0, w.1.as_slice()), "field round-trip");
    }

    // Any single-byte change is detectable: it either breaks the structure or
    // shifts the mod-256 checksum away from the declared trailer value (a
    // nonzero XOR strictly changes the byte, so the sum moves by a nonzero
    // delta mod 256; a flip inside the trailer corrupts the declared value or
    // the trailer shape itself).
    let mut corrupt = raw.clone();
    let pos = spec.flip_pos % corrupt.len();
    corrupt[pos] ^= spec.flip_mask;
    assert!(
        FrameCursor::parse(&corrupt).is_err(),
        "single-byte corruption at {pos} was accepted"
    );

    // Any strict prefix loses the exactly-trailing `10=NNN<SOH>` (the parser
    // demands the frame END at the trailer SOH) and must be rejected.
    let cut = spec.cut % raw.len();
    assert!(
        FrameCursor::parse(&raw[..cut]).is_err(),
        "truncated frame (cut at {cut}) was accepted"
    );
}

/// Phase 3 — STREAM: the real socket delimiter over adversarial chunking.
fn delimit_stream(data: &[u8]) {
    // Chunk size from the first byte (1..=64): small chunks split the
    // `10=NNN<SOH>` trailer across reads, exercising the scan-overlap rewind;
    // 4096 mirrors the reader's own slab for the single-read path.
    let chunk = 1 + usize::from(data.first().copied().unwrap_or(0)) % 64;
    for chunk in [chunk, 4096] {
        run_delimiter(data, chunk);
    }
}

fn run_delimiter(data: &[u8], chunk: usize) {
    let mut reader = FrameReader::new(ChunkedSlice { data, chunk });
    runtime().block_on(async {
        let mut consumed = 0usize;
        loop {
            match reader.next_frame().await {
                Ok(Some(frame)) => {
                    consumed += frame.len();
                    assert!(
                        consumed <= data.len(),
                        "delimiter invented bytes: {consumed} > {}",
                        data.len()
                    );
                    // Delimiter contract: every yielded frame ends with a
                    // well-formed `10=<3 digits><SOH>` trailer.
                    assert!(frame.len() >= 7, "frame shorter than a trailer");
                    let trailer = &frame[frame.len() - 7..];
                    assert_eq!(&trailer[..3], b"10=", "trailer tag");
                    assert!(
                        trailer[3..6].iter().all(u8::is_ascii_digit),
                        "trailer digits"
                    );
                    assert_eq!(trailer[6], SOH, "trailer SOH");
                    // Delimited is NOT validated — the full parse must uphold
                    // the no-panic / typed-Err contract on the carved bytes.
                    let _ = FrameCursor::parse(&frame);
                }
                Ok(None) => break, // clean EOF, no buffered partial frame
                Err(e) => {
                    // The only honest delimiter error: EOF mid-frame.
                    assert_eq!(
                        e.kind(),
                        std::io::ErrorKind::UnexpectedEof,
                        "unexpected delimiter error: {e}"
                    );
                    break;
                }
            }
        }
    });
}

/// Delivers a borrowed byte slice at most `chunk` bytes per read — a
/// deterministic stand-in for TCP segmentation so the delimiter's cross-read
/// trailer-scan overlap is exercised. Pure harness shim; no product logic.
struct ChunkedSlice<'a> {
    data: &'a [u8],
    chunk: usize,
}

impl AsyncRead for ChunkedSlice<'_> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let n = self.chunk.min(self.data.len()).min(buf.remaining());
        let (head, tail) = self.data.split_at(n);
        buf.put_slice(head);
        self.data = tail;
        Poll::Ready(Ok(()))
    }
}

/// One lazily-built current-thread runtime for the whole campaign — the
/// delimiter is async-edge code; building a runtime per exec would dominate
/// fuzzing throughput.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("current-thread runtime")
    })
}
