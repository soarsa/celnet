//! Blue-green upgrade **state handoff** (`docs/ARCHITECTURE.md` §5; ADR-0007).
//!
//! To upgrade a stateful low-latency engine with zero in-flight loss, a freshly
//! started process must take over the *running book and market state* of the
//! outgoing process. This module serializes that live state to a flat byte
//! buffer and restores it, so the successor reprices identically.
//!
//! # Single, current, un-versioned format (ADR-0007)
//!
//! Per guardrail #9 and ADR-0007 there is **exactly one** wire format and **no
//! `schema_version` field**: upgrades deploy a single uniform version, so there
//! is no N/N-1 negotiation window to support. The format is a deterministic
//! little-endian encoding (fixed-width integers; IEEE-754 `f64` bit patterns)
//! with a single fixed magic tag that namespaces the payload and lets a
//! mismatched/corrupt buffer fail fast — it is a content tag, not a version.
//!
//! # What is handed off
//!
//! * the [`BookState`] — every booked option line;
//! * the live [`MarketState`] essentials — spot, the two rates, vol-time, the
//!   resolved [`ConventionRecord`], and the calibrated smile (reconstructed from
//!   its three benchmark pillars, the form in which `celnet-surface` exposes it).
//!
//! Restoring yields a [`MarketState`] and [`BookState`] that reprice
//! bit-identically to the source (validated in the tests).

use celnet_conventions::ConventionRecord;
use celnet_surface::VannaVolgaSmile;
use celnet_types::{
    AtmConvention, Cut, DayCount, DeltaConvention, OptionType, PremiumStyle, Settlement,
};

use crate::rt::{BookEntry, BookState, MarketState};

/// The fixed content tag prefixing every handoff buffer. Not a version — a
/// single current format (ADR-0007); it only namespaces the payload so a foreign
/// or corrupt buffer is rejected before any field is decoded. ASCII `"CELNHND1"`.
const MAGIC: u64 = u64::from_le_bytes(*b"CELNHND1");

/// Errors decoding a handoff buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffError {
    /// The buffer is shorter than the bytes a field requires.
    Truncated,
    /// The leading content tag did not match [`MAGIC`].
    BadMagic,
    /// An enum discriminant byte was outside its valid range.
    BadDiscriminant,
    /// Trailing bytes remained after a complete record was decoded.
    TrailingBytes,
}

impl core::fmt::Display for HandoffError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            HandoffError::Truncated => "handoff buffer truncated",
            HandoffError::BadMagic => "handoff buffer has wrong content tag",
            HandoffError::BadDiscriminant => "handoff buffer has invalid enum discriminant",
            HandoffError::TrailingBytes => "handoff buffer has trailing bytes",
        };
        f.write_str(s)
    }
}

impl std::error::Error for HandoffError {}

/// A minimal append-only little-endian writer (heap `Vec`; handoff is an
/// off-the-hot-path, control-plane operation).
struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        // Serialize the exact IEEE-754 bit pattern so the restored value is
        // bit-identical (never a decimal round-trip).
        self.buf.extend_from_slice(&v.to_bits().to_le_bytes());
    }
}

/// A bounds-checked little-endian reader over a borrowed buffer.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], HandoffError> {
        let end = self.pos.checked_add(n).ok_or(HandoffError::Truncated)?;
        let slice = self.buf.get(self.pos..end).ok_or(HandoffError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8, HandoffError> {
        Ok(self.take(1)?[0])
    }
    fn u64(&mut self) -> Result<u64, HandoffError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(
            b.try_into().expect("took exactly 8 bytes"),
        ))
    }
    fn f64(&mut self) -> Result<f64, HandoffError> {
        Ok(f64::from_bits(self.u64()?))
    }
    fn finish(self) -> Result<(), HandoffError> {
        if self.pos == self.buf.len() {
            Ok(())
        } else {
            Err(HandoffError::TrailingBytes)
        }
    }
}

// ---- enum ⇄ byte codecs (single current contract) -------------------------

fn enc_option_type(v: OptionType) -> u8 {
    match v {
        OptionType::Call => 0,
        OptionType::Put => 1,
    }
}
fn dec_option_type(b: u8) -> Result<OptionType, HandoffError> {
    match b {
        0 => Ok(OptionType::Call),
        1 => Ok(OptionType::Put),
        _ => Err(HandoffError::BadDiscriminant),
    }
}

fn enc_delta(v: DeltaConvention) -> u8 {
    match v {
        DeltaConvention::SpotUnadjusted => 0,
        DeltaConvention::ForwardUnadjusted => 1,
        DeltaConvention::SpotPremiumAdjusted => 2,
        DeltaConvention::ForwardPremiumAdjusted => 3,
    }
}
fn dec_delta(b: u8) -> Result<DeltaConvention, HandoffError> {
    match b {
        0 => Ok(DeltaConvention::SpotUnadjusted),
        1 => Ok(DeltaConvention::ForwardUnadjusted),
        2 => Ok(DeltaConvention::SpotPremiumAdjusted),
        3 => Ok(DeltaConvention::ForwardPremiumAdjusted),
        _ => Err(HandoffError::BadDiscriminant),
    }
}

fn enc_atm(v: AtmConvention) -> u8 {
    match v {
        AtmConvention::AtmForward => 0,
        AtmConvention::DeltaNeutralStraddle => 1,
    }
}
fn dec_atm(b: u8) -> Result<AtmConvention, HandoffError> {
    match b {
        0 => Ok(AtmConvention::AtmForward),
        1 => Ok(AtmConvention::DeltaNeutralStraddle),
        _ => Err(HandoffError::BadDiscriminant),
    }
}

fn enc_premium(v: PremiumStyle) -> u8 {
    match v {
        PremiumStyle::DomesticPips => 0,
        PremiumStyle::PercentForeign => 1,
        PremiumStyle::PercentDomestic => 2,
        PremiumStyle::ForeignPips => 3,
    }
}
fn dec_premium(b: u8) -> Result<PremiumStyle, HandoffError> {
    match b {
        0 => Ok(PremiumStyle::DomesticPips),
        1 => Ok(PremiumStyle::PercentForeign),
        2 => Ok(PremiumStyle::PercentDomestic),
        3 => Ok(PremiumStyle::ForeignPips),
        _ => Err(HandoffError::BadDiscriminant),
    }
}

fn enc_cut(v: Cut) -> u8 {
    match v {
        Cut::NewYork1000 => 0,
        Cut::Tokyo1500 => 1,
    }
}
fn dec_cut(b: u8) -> Result<Cut, HandoffError> {
    match b {
        0 => Ok(Cut::NewYork1000),
        1 => Ok(Cut::Tokyo1500),
        _ => Err(HandoffError::BadDiscriminant),
    }
}

fn enc_daycount(v: DayCount) -> u8 {
    match v {
        DayCount::Act365Fixed => 0,
        DayCount::Act360 => 1,
    }
}
fn dec_daycount(b: u8) -> Result<DayCount, HandoffError> {
    match b {
        0 => Ok(DayCount::Act365Fixed),
        1 => Ok(DayCount::Act360),
        _ => Err(HandoffError::BadDiscriminant),
    }
}

fn enc_settlement(v: Settlement) -> u8 {
    match v {
        Settlement::Deliverable => 0,
        Settlement::NonDeliverable => 1,
    }
}
fn dec_settlement(b: u8) -> Result<Settlement, HandoffError> {
    match b {
        0 => Ok(Settlement::Deliverable),
        1 => Ok(Settlement::NonDeliverable),
        _ => Err(HandoffError::BadDiscriminant),
    }
}

fn write_conventions(w: &mut Writer, c: &ConventionRecord) {
    w.u8(enc_delta(c.delta));
    w.u8(enc_atm(c.atm));
    w.u8(enc_premium(c.premium_style));
    w.u8(enc_cut(c.cut));
    w.u8(enc_daycount(c.day_count_vol));
    w.u8(enc_daycount(c.day_count_accrual_for));
    w.u8(enc_daycount(c.day_count_accrual_dom));
    w.u8(enc_settlement(c.settlement));
}

fn read_conventions(r: &mut Reader<'_>) -> Result<ConventionRecord, HandoffError> {
    let delta = dec_delta(r.u8()?)?;
    let atm = dec_atm(r.u8()?)?;
    let premium_style = dec_premium(r.u8()?)?;
    let cut = dec_cut(r.u8()?)?;
    let day_count_vol = dec_daycount(r.u8()?)?;
    let day_count_accrual_for = dec_daycount(r.u8()?)?;
    let day_count_accrual_dom = dec_daycount(r.u8()?)?;
    let settlement = dec_settlement(r.u8()?)?;
    Ok(ConventionRecord::new(
        delta,
        atm,
        premium_style,
        cut,
        day_count_vol,
        day_count_accrual_for,
        day_count_accrual_dom,
        settlement,
    ))
}

// ---- public API -----------------------------------------------------------

/// Serialize the live engine state (the [`MarketState`] and the [`BookState`])
/// to a self-describing, un-versioned byte buffer (ADR-0007).
///
/// The layout is: magic tag · market scalars (`f64` bit patterns) · convention
/// record (8 discriminant bytes) · the smile's three benchmark pillars and
/// reference forward/time · the book (`u64` length + fixed-width entries). A
/// matching [`restore_state`] reconstructs an identical [`MarketState`] /
/// [`BookState`].
#[must_use]
pub fn serialize_state(market: &MarketState, book: &BookState) -> Vec<u8> {
    // 8 (magic) + 4*8 (market scalars) + 8 (conventions) + smile + book.
    let mut w = Writer::with_capacity(64 + book.entries.len() * 32);
    w.u64(MAGIC);

    // Market scalars.
    w.f64(market.spot);
    w.f64(market.r_dom);
    w.f64(market.r_for);
    w.f64(market.t);

    // Conventions.
    write_conventions(&mut w, &market.conventions);

    // Smile: three benchmark pillars + reference forward/time. This is the exact
    // state from which `VannaVolgaSmile::new` reconstructs the identical smile.
    let strikes = market.smile.benchmark_strikes();
    let vols = market.smile.benchmark_vols();
    for &k in &strikes {
        w.f64(k);
    }
    for &v in &vols {
        w.f64(v);
    }
    w.f64(market.smile.forward());
    w.f64(market.smile.reference_t());

    // Book.
    w.u64(book.entries.len() as u64);
    for e in &book.entries {
        w.u64(e.id);
        w.u8(enc_option_type(e.option_type));
        w.f64(e.strike);
        w.f64(e.notional);
    }

    w.buf
}

/// Restore the live engine state from a buffer produced by [`serialize_state`].
///
/// # Errors
///
/// Returns [`HandoffError`] if the content tag is wrong, the buffer is
/// truncated, an enum discriminant is invalid, or trailing bytes remain.
pub fn restore_state(bytes: &[u8]) -> Result<(MarketState, BookState), HandoffError> {
    let mut r = Reader::new(bytes);

    if r.u64()? != MAGIC {
        return Err(HandoffError::BadMagic);
    }

    let spot = r.f64()?;
    let r_dom = r.f64()?;
    let r_for = r.f64()?;
    let t = r.f64()?;

    let conventions = read_conventions(&mut r)?;

    let strikes = [r.f64()?, r.f64()?, r.f64()?];
    let vols = [r.f64()?, r.f64()?, r.f64()?];
    let smile_forward = r.f64()?;
    let smile_t = r.f64()?;
    let smile = VannaVolgaSmile::new(strikes, vols, smile_forward, smile_t);

    let n = r.u64()? as usize;
    let mut entries = Vec::with_capacity(n);
    for _ in 0..n {
        let id = r.u64()?;
        let option_type = dec_option_type(r.u8()?)?;
        let strike = r.f64()?;
        let notional = r.f64()?;
        entries.push(BookEntry {
            id,
            option_type,
            strike,
            notional,
        });
    }

    r.finish()?;

    let market = MarketState {
        spot,
        r_dom,
        r_for,
        t,
        conventions,
        smile,
    };
    Ok((market, BookState { entries }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{PriceRequest, PricingCore};
    use crate::testing::market_state;
    use celnet_core::is_close;
    use celnet_types::{CcyPair, Tenor};

    fn conv() -> ConventionRecord {
        celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record
    }

    fn sample_book() -> BookState {
        let mut b = BookState::new();
        b.push(BookEntry {
            id: 11,
            option_type: OptionType::Call,
            strike: 1.12,
            notional: 1_000_000.0,
        });
        b.push(BookEntry {
            id: 12,
            option_type: OptionType::Put,
            strike: 1.05,
            notional: -500_000.0,
        });
        b
    }

    #[test]
    fn roundtrip_restores_identical_state() {
        let m = market_state(1.10, 0.105, 0.015, 0.0035, conv());
        let b = sample_book();
        let bytes = serialize_state(&m, &b);
        let (m2, b2) = restore_state(&bytes).expect("decode");

        // Market scalars bit-identical.
        assert_eq!(m2.spot.to_bits(), m.spot.to_bits());
        assert_eq!(m2.r_dom.to_bits(), m.r_dom.to_bits());
        assert_eq!(m2.r_for.to_bits(), m.r_for.to_bits());
        assert_eq!(m2.t.to_bits(), m.t.to_bits());
        assert_eq!(m2.conventions, m.conventions);

        // Smile pillars bit-identical.
        for i in 0..3 {
            assert_eq!(
                m2.smile.benchmark_strikes()[i].to_bits(),
                m.smile.benchmark_strikes()[i].to_bits()
            );
            assert_eq!(
                m2.smile.benchmark_vols()[i].to_bits(),
                m.smile.benchmark_vols()[i].to_bits()
            );
        }
        // Book identical.
        assert_eq!(b2, b);
    }

    #[test]
    fn restored_state_reprices_identically() {
        // The successor process reprices the whole book bit-for-bit.
        let m = market_state(1.10, 0.105, 0.015, 0.0035, conv());
        let b = sample_book();
        let bytes = serialize_state(&m, &b);
        let (m2, b2) = restore_state(&bytes).unwrap();

        let mut src = PricingCore::new(m);
        let mut dst = PricingCore::new(m2);
        for (e1, e2) in b.entries.iter().zip(b2.entries.iter()) {
            let r1 = src.price(PriceRequest::new(e1.id, e1.option_type, e1.strike));
            let r2 = dst.price(PriceRequest::new(e2.id, e2.option_type, e2.strike));
            // Identical to the bit (same inputs, same deterministic math).
            assert_eq!(r1.greeks.price.to_bits(), r2.greeks.price.to_bits());
            assert_eq!(
                r1.greeks.delta_spot.to_bits(),
                r2.greeks.delta_spot.to_bits()
            );
            assert_eq!(r1.vol.to_bits(), r2.vol.to_bits());
            assert!(is_close(r1.greeks.vega, r2.greeks.vega, 0.0, 0.0));
        }
    }

    #[test]
    fn rejects_bad_magic() {
        let m = market_state(1.10, 0.105, 0.015, 0.0035, conv());
        let mut bytes = serialize_state(&m, &sample_book());
        bytes[0] ^= 0xFF;
        assert_eq!(restore_state(&bytes).unwrap_err(), HandoffError::BadMagic);
    }

    #[test]
    fn rejects_truncated() {
        let m = market_state(1.10, 0.105, 0.015, 0.0035, conv());
        let bytes = serialize_state(&m, &sample_book());
        assert_eq!(
            restore_state(&bytes[..bytes.len() - 3]).unwrap_err(),
            HandoffError::Truncated
        );
    }

    #[test]
    fn rejects_trailing_bytes() {
        let m = market_state(1.10, 0.105, 0.015, 0.0035, conv());
        let mut bytes = serialize_state(&m, &sample_book());
        bytes.push(0);
        assert_eq!(
            restore_state(&bytes).unwrap_err(),
            HandoffError::TrailingBytes
        );
    }
}
