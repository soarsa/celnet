//! Zero-copy FIX 4.4 framing: `SOH`-delimited `tag=value` runs with
//! `BodyLength(9)` / `CheckSum(10)` compute and validation.
//!
//! A FIX message on the wire is a flat run of `tag=value` fields separated by
//! the ASCII `SOH` (`0x01`) byte. The standard mandates a fixed prologue and
//! epilogue:
//!
//! ```text
//! 8=FIX.4.4 | 9=<BodyLength> | <body fields...> | 10=<CheckSum> |
//! ```
//!
//! * `BodyLength` (tag 9) is the byte count from the first byte **after** the
//!   `9=...SOH` field up to and **including** the `SOH` that precedes the
//!   `10=` checksum field.
//! * `CheckSum` (tag 10) is the sum of every byte up to and including the `SOH`
//!   before `10=`, taken modulo 256 and rendered as a zero-padded 3-digit
//!   decimal.
//!
//! [`FrameCursor`] parses such a buffer *in place* — it borrows the input
//! `&[u8]` and exposes typed accessors over borrowed field slices. No field is
//! copied and nothing is allocated per message, matching the edge latency
//! discipline (`docs/ARCHITECTURE.md` §3). The design follows the
//! dictionary-driven tag-value-slice model popularised by the `fefix` crate
//! (cited per guardrail; no runtime dependency on it).

/// ASCII `SOH` (start-of-heading), the FIX field separator.
pub const SOH: u8 = 0x01;
/// ASCII `=`, the FIX tag/value separator.
pub const EQ: u8 = b'=';

/// FIX tag number for `BeginString`.
pub const TAG_BEGIN_STRING: u32 = 8;
/// FIX tag number for `BodyLength`.
pub const TAG_BODY_LENGTH: u32 = 9;
/// FIX tag number for `MsgType`.
pub const TAG_MSG_TYPE: u32 = 35;
/// FIX tag number for `CheckSum`.
pub const TAG_CHECK_SUM: u32 = 10;

/// The FIXT/4.4 `BeginString` literal.
pub const BEGIN_STRING_FIX44: &[u8] = b"FIX.4.4";

/// A framing-level parse or validation error. Every variant is recoverable —
/// the parser never panics on malformed input (guardrail: malformed frames
/// rejected without panic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The buffer is shorter than the minimal valid FIX message.
    TooShort,
    /// A `tag=value` field was missing its `=` separator.
    MissingEquals,
    /// A field was not terminated by `SOH`.
    UnterminatedField,
    /// A tag was empty or contained a non-digit byte.
    BadTag,
    /// The message did not begin with `8=` (`BeginString`).
    MissingBeginString,
    /// The body contained no `MsgType` (tag 35).
    MissingMsgType,
    /// The `BeginString` value was not a supported FIX version.
    UnsupportedBeginString,
    /// The second field was not `9=` (`BodyLength`).
    MissingBodyLength,
    /// `BodyLength` was not a valid non-negative integer.
    BadBodyLength,
    /// The declared `BodyLength` did not match the actual body byte count.
    BodyLengthMismatch {
        /// The value declared in tag 9.
        declared: usize,
        /// The byte count actually measured between the fields.
        actual: usize,
    },
    /// The trailer field was not `10=` (`CheckSum`), or was absent.
    MissingCheckSum,
    /// `CheckSum` was not a 3-digit value.
    BadCheckSum,
    /// The declared checksum did not match the computed one.
    CheckSumMismatch {
        /// The value declared in tag 10.
        declared: u8,
        /// The checksum actually computed over the bytes.
        computed: u8,
    },
}

/// A single borrowed `tag=value` field within a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field<'a> {
    /// The numeric tag.
    pub tag: u32,
    /// The raw value bytes (excluding the trailing `SOH`).
    pub value: &'a [u8],
}

/// The modulo-256 byte sum FIX uses as its checksum.
///
/// Summing into a `u32` and masking avoids per-byte `u8` wrapping branches; the
/// result is identical to the standard's `sum mod 256`.
#[must_use]
pub fn checksum(bytes: &[u8]) -> u8 {
    let mut acc: u32 = 0;
    for &b in bytes {
        acc = acc.wrapping_add(u32::from(b));
    }
    (acc & 0xFF) as u8
}

/// Parse an unsigned decimal integer from raw ASCII bytes.
///
/// Returns `None` on an empty slice or any non-digit byte (no leading `+`/`-`,
/// no whitespace) — FIX integer fields are strict.
#[must_use]
pub fn parse_uint(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    let mut v: u64 = 0;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add(u64::from(b - b'0'))?;
    }
    Some(v)
}

/// A zero-copy cursor over a single, already-delimited FIX frame.
///
/// Construct via [`FrameCursor::parse`], which validates the prologue
/// (`8=`/`9=`), the `BodyLength`, and the `CheckSum` before returning. After
/// that the body fields are iterated lazily with [`FrameCursor::fields`] or
/// looked up by tag with [`FrameCursor::get`]; both borrow from the original
/// buffer with no allocation.
#[derive(Debug, Clone, Copy)]
pub struct FrameCursor<'a> {
    /// The whole frame including prologue and trailer.
    raw: &'a [u8],
    /// Byte offset of the first body field (after `9=<n>SOH`).
    body_start: usize,
    /// Byte offset one past the last body field (the start of `10=`).
    body_end: usize,
    /// The parsed `MsgType` (tag 35) value bytes.
    msg_type: &'a [u8],
}

impl<'a> FrameCursor<'a> {
    /// Parse and fully validate a single FIX frame.
    ///
    /// The buffer must contain exactly one message: `8=...` through the
    /// `10=...SOH` trailer. `BodyLength` and `CheckSum` are both checked.
    ///
    /// # Errors
    /// Returns a [`FrameError`] for any structural, length, or checksum fault;
    /// never panics.
    pub fn parse(raw: &'a [u8]) -> Result<Self, FrameError> {
        // 8=FIX.4.4<SOH> is 10 bytes; 9=0<SOH> is 4; 10=000<SOH> is 7 — a
        // valid frame cannot be shorter than the prologue + trailer.
        if raw.len() < 21 {
            return Err(FrameError::TooShort);
        }

        // --- 8 = BeginString ---
        let (begin, after_begin) = take_field(raw, 0)?;
        if begin.tag != TAG_BEGIN_STRING {
            return Err(FrameError::MissingBeginString);
        }
        if begin.value != BEGIN_STRING_FIX44 {
            return Err(FrameError::UnsupportedBeginString);
        }

        // --- 9 = BodyLength ---
        let (blen_field, body_start) = take_field(raw, after_begin)?;
        if blen_field.tag != TAG_BODY_LENGTH {
            return Err(FrameError::MissingBodyLength);
        }
        let declared_body_len =
            parse_uint(blen_field.value).ok_or(FrameError::BadBodyLength)? as usize;

        // The body runs from `body_start` up to and including the SOH before
        // `10=`. Locate the checksum field: it is the last field and starts
        // with "10=".
        let cs_field_start = find_checksum_field(raw, body_start)?;
        // Body length is bytes from body_start to cs_field_start inclusive of
        // the SOH preceding 10= — i.e. exactly `cs_field_start - body_start`.
        let actual_body_len = cs_field_start - body_start;
        if actual_body_len != declared_body_len {
            return Err(FrameError::BodyLengthMismatch {
                declared: declared_body_len,
                actual: actual_body_len,
            });
        }

        // --- 10 = CheckSum (trailer) ---
        let (cs_field, after_cs) = take_field(raw, cs_field_start)?;
        if cs_field.tag != TAG_CHECK_SUM {
            return Err(FrameError::MissingCheckSum);
        }
        if cs_field.value.len() != 3 {
            return Err(FrameError::BadCheckSum);
        }
        let declared_cs = parse_uint(cs_field.value).ok_or(FrameError::BadCheckSum)? as u8;
        // The frame must end exactly at the trailer SOH.
        if after_cs != raw.len() {
            return Err(FrameError::MissingCheckSum);
        }
        let computed_cs = checksum(&raw[..cs_field_start]);
        if computed_cs != declared_cs {
            return Err(FrameError::CheckSumMismatch {
                declared: declared_cs,
                computed: computed_cs,
            });
        }

        // --- MsgType (35) must be present in the body ---
        let body_end = cs_field_start;
        let mut msg_type: Option<&[u8]> = None;
        let mut off = body_start;
        while off < body_end {
            let (f, next) = take_field(raw, off)?;
            if f.tag == TAG_MSG_TYPE {
                msg_type = Some(f.value);
                break;
            }
            off = next;
        }
        let msg_type = msg_type.ok_or(FrameError::MissingMsgType)?;

        Ok(FrameCursor {
            raw,
            body_start,
            body_end,
            msg_type,
        })
    }

    /// The `MsgType` (tag 35) value bytes.
    #[must_use]
    pub fn msg_type(&self) -> &'a [u8] {
        self.msg_type
    }

    /// Iterate the body fields (everything between `9=` and `10=`), in wire
    /// order, including repeating-group members.
    pub fn fields(&self) -> FieldIter<'a> {
        FieldIter {
            raw: self.raw,
            off: self.body_start,
            end: self.body_end,
        }
    }

    /// The raw value bytes for the **first** occurrence of `tag` in the body,
    /// or `None` if absent. O(n) scan — repeating groups are walked with
    /// [`FrameCursor::fields`].
    #[must_use]
    pub fn get(&self, tag: u32) -> Option<&'a [u8]> {
        self.fields().find(|f| f.tag == tag).map(|f| f.value)
    }

    /// The whole frame's raw bytes (prologue + body + trailer).
    #[must_use]
    pub fn raw(&self) -> &'a [u8] {
        self.raw
    }
}

/// Lazy iterator over the body fields of a [`FrameCursor`].
#[derive(Debug, Clone)]
pub struct FieldIter<'a> {
    raw: &'a [u8],
    off: usize,
    end: usize,
}

impl<'a> Iterator for FieldIter<'a> {
    type Item = Field<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.off >= self.end {
            return None;
        }
        // `take_field` only fails on malformed input, which `FrameCursor::parse`
        // already rejected; treat any residual fault as end-of-iteration rather
        // than panicking.
        let (field, next) = take_field(self.raw, self.off).ok()?;
        if next > self.end {
            // The checksum field boundary would be crossed — stop cleanly.
            return None;
        }
        self.off = next;
        Some(field)
    }
}

/// Parse one `tag=value<SOH>` field starting at `off`; return the field and the
/// offset of the next field.
fn take_field(raw: &[u8], off: usize) -> Result<(Field<'_>, usize), FrameError> {
    if off >= raw.len() {
        return Err(FrameError::TooShort);
    }
    // Find the SOH that terminates this field.
    let rel_soh = raw[off..]
        .iter()
        .position(|&b| b == SOH)
        .ok_or(FrameError::UnterminatedField)?;
    let field_bytes = &raw[off..off + rel_soh];
    let eq = field_bytes
        .iter()
        .position(|&b| b == EQ)
        .ok_or(FrameError::MissingEquals)?;
    let tag_bytes = &field_bytes[..eq];
    let value = &field_bytes[eq + 1..];
    let tag = parse_uint(tag_bytes).ok_or(FrameError::BadTag)? as u32;
    if tag_bytes.is_empty() {
        return Err(FrameError::BadTag);
    }
    Ok((Field { tag, value }, off + rel_soh + 1))
}

/// Locate the byte offset where the `10=` (CheckSum) field begins. The checksum
/// is always the final field, so we scan field boundaries until the field tag
/// is 10.
fn find_checksum_field(raw: &[u8], body_start: usize) -> Result<usize, FrameError> {
    let mut off = body_start;
    while off < raw.len() {
        let (field, next) = take_field(raw, off)?;
        if field.tag == TAG_CHECK_SUM {
            return Ok(off);
        }
        off = next;
    }
    Err(FrameError::MissingCheckSum)
}

/// A reusable owned encoder that lays out a FIX frame into a byte buffer with a
/// correct `BodyLength` and `CheckSum`.
///
/// The encoder builds the body fields first (so their length is known), then
/// prepends the `8=`/`9=` prologue and appends the `10=` trailer. It is the
/// only allocation in the engine and lives on the async edge, never the hot
/// path. The buffer can be reused across messages via [`FrameEncoder::clear`].
#[derive(Debug, Default, Clone)]
pub struct FrameEncoder {
    body: Vec<u8>,
}

impl FrameEncoder {
    /// A fresh encoder with an empty body buffer.
    #[must_use]
    pub fn new() -> Self {
        Self { body: Vec::new() }
    }

    /// Reset the body buffer for reuse without freeing its capacity.
    pub fn clear(&mut self) {
        self.body.clear();
    }

    /// Append a `tag=value<SOH>` field to the body. `MsgType` (35) is a normal
    /// body field and must be pushed first by the message builders.
    pub fn push(&mut self, tag: u32, value: &[u8]) {
        push_uint(&mut self.body, u64::from(tag));
        self.body.push(EQ);
        self.body.extend_from_slice(value);
        self.body.push(SOH);
    }

    /// Append an integer-valued field.
    pub fn push_int(&mut self, tag: u32, value: i64) {
        push_uint(&mut self.body, u64::from(tag));
        self.body.push(EQ);
        if value < 0 {
            self.body.push(b'-');
            push_uint(&mut self.body, value.unsigned_abs());
        } else {
            push_uint(&mut self.body, value as u64);
        }
        self.body.push(SOH);
    }

    /// Finalise the frame: prepend `8=FIX.4.4`/`9=<len>`, append `10=<cs>`, and
    /// return the complete on-wire bytes. The encoder's body buffer is left
    /// intact (call [`FrameEncoder::clear`] to reuse).
    #[must_use]
    pub fn finish(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.body.len() + 24);
        // 8=FIX.4.4<SOH>
        out.extend_from_slice(b"8=");
        out.extend_from_slice(BEGIN_STRING_FIX44);
        out.push(SOH);
        // 9=<bodylen><SOH>
        out.extend_from_slice(b"9=");
        push_uint(&mut out, self.body.len() as u64);
        out.push(SOH);
        // body (includes 35=...)
        out.extend_from_slice(&self.body);
        // 10=<checksum><SOH>
        let cs = checksum(&out);
        out.extend_from_slice(b"10=");
        push_3digit(&mut out, cs);
        out.push(SOH);
        out
    }
}

/// Append the decimal digits of `v` to `buf` without an intermediate `String`.
fn push_uint(buf: &mut Vec<u8>, v: u64) {
    if v == 0 {
        buf.push(b'0');
        return;
    }
    let mut tmp = [0u8; 20];
    let mut i = tmp.len();
    let mut n = v;
    while n > 0 {
        i -= 1;
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    buf.extend_from_slice(&tmp[i..]);
}

/// Append a zero-padded 3-digit decimal (the FIX checksum format).
fn push_3digit(buf: &mut Vec<u8>, v: u8) {
    buf.push(b'0' + (v / 100));
    buf.push(b'0' + (v / 10 % 10));
    buf.push(b'0' + (v % 10));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_simple() -> Vec<u8> {
        let mut e = FrameEncoder::new();
        e.push(TAG_MSG_TYPE, b"0"); // heartbeat
        e.push(49, b"CELNET");
        e.push(56, b"CPARTY");
        e.push_int(34, 1);
        e.finish()
    }

    #[test]
    fn roundtrip_simple() {
        let raw = build_simple();
        let f = FrameCursor::parse(&raw).expect("valid frame");
        assert_eq!(f.msg_type(), b"0");
        assert_eq!(f.get(49), Some(&b"CELNET"[..]));
        assert_eq!(f.get(56), Some(&b"CPARTY"[..]));
        assert_eq!(f.get(34), Some(&b"1"[..]));
        assert_eq!(f.get(999), None);
    }

    #[test]
    fn checksum_is_mod_256() {
        assert_eq!(checksum(b""), 0);
        assert_eq!(checksum(&[255, 1]), 0);
        assert_eq!(checksum(&[255, 2]), 1);
    }

    #[test]
    fn rejects_corrupted_checksum() {
        let mut raw = build_simple();
        // Flip a body byte: checksum must now mismatch.
        let pos = raw.iter().position(|&b| b == b'C').unwrap();
        raw[pos] = b'X';
        let err = FrameCursor::parse(&raw).unwrap_err();
        assert!(matches!(err, FrameError::CheckSumMismatch { .. }));
    }

    #[test]
    fn rejects_bad_body_length() {
        let raw = build_simple();
        // Re-encode with a deliberately wrong 9= value by hand.
        let s = String::from_utf8(raw).unwrap();
        // The original 9= field; bump its number.
        let corrupted = s.replacen("9=", "9=99", 1);
        let err = FrameCursor::parse(corrupted.as_bytes()).unwrap_err();
        assert!(matches!(
            err,
            FrameError::BodyLengthMismatch { .. } | FrameError::BadBodyLength
        ));
    }

    #[test]
    fn rejects_missing_begin_string() {
        let raw = b"35=0\x0149=A\x0110=000\x01";
        let err = FrameCursor::parse(raw).unwrap_err();
        assert!(matches!(
            err,
            FrameError::TooShort | FrameError::MissingBeginString
        ));
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut e = FrameEncoder::new();
        e.push(TAG_MSG_TYPE, b"0");
        let mut raw = e.finish();
        // Rewrite FIX.4.4 -> FIX.4.2 and fix nothing else: must be rejected.
        let s = String::from_utf8(raw.clone())
            .unwrap()
            .replace("4.4", "4.2");
        raw = s.into_bytes();
        let err = FrameCursor::parse(&raw).unwrap_err();
        assert!(matches!(
            err,
            FrameError::UnsupportedBeginString | FrameError::CheckSumMismatch { .. }
        ));
    }

    #[test]
    fn no_panic_on_arbitrary_garbage() {
        // A battery of malformed inputs must all return Err, never panic.
        let cases: &[&[u8]] = &[
            b"",
            b"\x01",
            b"8=",
            b"8=FIX.4.4\x01",
            b"8=FIX.4.4\x019=\x01",
            b"=\x01=\x01=\x01",
            b"8=FIX.4.4\x019=abc\x0135=0\x0110=000\x01",
            b"\x01\x01\x01\x01\x01\x01\x01\x01\x01\x01\x01\x01\x01\x01\x01\x01\x01\x01\x01\x01\x01\x01",
        ];
        for c in cases {
            let _ = FrameCursor::parse(c); // must not panic
        }
    }

    #[test]
    fn parse_uint_strict() {
        assert_eq!(parse_uint(b"0"), Some(0));
        assert_eq!(parse_uint(b"123"), Some(123));
        assert_eq!(parse_uint(b""), None);
        assert_eq!(parse_uint(b"-1"), None);
        assert_eq!(parse_uint(b"1a"), None);
        assert_eq!(parse_uint(b" 1"), None);
    }
}
