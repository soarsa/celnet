//! `celnet-sbe` — Simple Binary Encoding (SBE) flyweight codec.
//!
//! Provides zero-allocation, fixed-offset, forward-compatible message serialization
//! and direct-buffer flyweight decoders following the FIX Trading Community SBE standard.
//!
//! Designed for the ultra-low-latency IPC tier and multicast network streaming,
//! achieving sub-10ns serialization and zero heap allocations.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod direct_buffer;
pub mod dispatcher;
pub mod gateway;
pub mod multicast;

pub use direct_buffer::{DirectBuffer, MutableDirectBuffer};
pub use dispatcher::{SbeDispatcher, SbeMessageRef};

use thiserror::Error;

/// Current SBE Schema ID for Celnet.
pub const CELNET_SBE_SCHEMA_ID: u16 = 1;
/// Current SBE Schema Version.
pub const CELNET_SBE_SCHEMA_VERSION: u16 = 1;
/// Standard 8-byte SBE Header length.
pub const SBE_HEADER_SIZE: usize = 8;

/// Template IDs for Celnet messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum TemplateId {
    /// Spot price tick.
    PriceTick = 101,
    /// Two-way option quote with full Greek sensitivities.
    OptionQuote = 102,
    /// Inbound RFQ request intent.
    QuoteRequest = 103,
    /// Confirmed execution report.
    ExecutionReport = 104,
    /// Order book level update.
    MarketDepthLevel = 105,
}

impl TemplateId {
    /// Convert raw u16 to TemplateId.
    pub fn from_u16(val: u16) -> Option<Self> {
        match val {
            101 => Some(Self::PriceTick),
            102 => Some(Self::OptionQuote),
            103 => Some(Self::QuoteRequest),
            104 => Some(Self::ExecutionReport),
            105 => Some(Self::MarketDepthLevel),
            _ => None,
        }
    }
}

/// SBE serialization and framing errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SbeError {
    /// Provided buffer is smaller than required frame size.
    #[error("buffer too short: expected at least {expected} bytes, got {actual}")]
    BufferTooShort {
        /// Expected minimum buffer size.
        expected: usize,
        /// Actual provided buffer size.
        actual: usize,
    },
    /// Unrecognized template identifier.
    #[error("unknown template ID: {0}")]
    UnknownTemplateId(u16),
    /// Schema ID mismatch.
    #[error("schema ID mismatch: expected {expected}, got {actual}")]
    SchemaIdMismatch {
        /// Expected schema ID.
        expected: u16,
        /// Actual schema ID found.
        actual: u16,
    },
    /// Invalid enumeration tag.
    #[error("invalid enum value: {0}")]
    InvalidEnumValue(u8),
}

/// Order / quote trading side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum Side {
    /// Buy side (bid).
    Buy = 1,
    /// Sell side (offer).
    Sell = 2,
    /// Two-way market.
    TwoWay = 3,
}

impl Side {
    /// Parse u8 tag to Side.
    pub fn from_u8(v: u8) -> Result<Self, SbeError> {
        match v {
            1 => Ok(Self::Buy),
            2 => Ok(Self::Sell),
            3 => Ok(Self::TwoWay),
            _ => Err(SbeError::InvalidEnumValue(v)),
        }
    }
}

/// Execution status reported by desk or exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum ExecutionStatus {
    /// New order placed.
    New = 0,
    /// Order filled / executed.
    Filled = 1,
    /// Order rejected.
    Rejected = 2,
    /// Quote or order expired.
    Expired = 3,
}

impl ExecutionStatus {
    /// Parse u8 tag to ExecutionStatus.
    pub fn from_u8(v: u8) -> Result<Self, SbeError> {
        match v {
            0 => Ok(Self::New),
            1 => Ok(Self::Filled),
            2 => Ok(Self::Rejected),
            3 => Ok(Self::Expired),
            _ => Err(SbeError::InvalidEnumValue(v)),
        }
    }
}

/// Canonical SBE 8-byte Message Header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageHeader {
    /// Block length of the fixed fields body.
    pub block_length: u16,
    /// Message template identifier.
    pub template_id: u16,
    /// Schema identifier.
    pub schema_id: u16,
    /// Schema version.
    pub version: u16,
}

impl MessageHeader {
    /// Encode header into the beginning of a buffer.
    #[inline(always)]
    pub fn encode(&self, dest: &mut [u8]) -> Result<(), SbeError> {
        if dest.len() < SBE_HEADER_SIZE {
            return Err(SbeError::BufferTooShort {
                expected: SBE_HEADER_SIZE,
                actual: dest.len(),
            });
        }
        dest[0..2].copy_from_slice(&self.block_length.to_le_bytes());
        dest[2..4].copy_from_slice(&self.template_id.to_le_bytes());
        dest[4..6].copy_from_slice(&self.schema_id.to_le_bytes());
        dest[6..8].copy_from_slice(&self.version.to_le_bytes());
        Ok(())
    }

    /// Decode header from buffer prefix.
    #[inline(always)]
    pub fn decode(src: &[u8]) -> Result<Self, SbeError> {
        if src.len() < SBE_HEADER_SIZE {
            return Err(SbeError::BufferTooShort {
                expected: SBE_HEADER_SIZE,
                actual: src.len(),
            });
        }
        let block_length = u16::from_le_bytes(src[0..2].try_into().unwrap());
        let template_id = u16::from_le_bytes(src[2..4].try_into().unwrap());
        let schema_id = u16::from_le_bytes(src[4..6].try_into().unwrap());
        let version = u16::from_le_bytes(src[6..8].try_into().unwrap());
        Ok(Self {
            block_length,
            template_id,
            schema_id,
            version,
        })
    }
}

// -----------------------------------------------------------------------------
// 1. PriceTick (Template 101)
// -----------------------------------------------------------------------------
/// Block length for PriceTick fixed fields.
pub const PRICE_TICK_BLOCK_LENGTH: usize = 40;
/// Total encoded frame length for PriceTick.
pub const PRICE_TICK_TOTAL_SIZE: usize = SBE_HEADER_SIZE + PRICE_TICK_BLOCK_LENGTH;

/// Spot market data price tick DTO.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PriceTick {
    /// UTC timestamp in nanoseconds since Unix epoch.
    pub epoch_nanos: i64,
    /// Currency pair numeric identifier.
    pub pair_id: u32,
    /// Status flags (e.g. tradable, indicative).
    pub flags: u32,
    /// Best bid price.
    pub bid: f64,
    /// Best ask / offer price.
    pub ask: f64,
}

/// SBE Flyweight decoder for PriceTick.
#[derive(Debug)]
pub struct PriceTickFlyweight<'a> {
    buffer: &'a [u8],
}

impl<'a> PriceTickFlyweight<'a> {
    /// Wrap a byte buffer after validating header and frame length.
    #[inline(always)]
    pub fn wrap(buffer: &'a [u8]) -> Result<Self, SbeError> {
        if buffer.len() < PRICE_TICK_TOTAL_SIZE {
            return Err(SbeError::BufferTooShort {
                expected: PRICE_TICK_TOTAL_SIZE,
                actual: buffer.len(),
            });
        }
        let header = MessageHeader::decode(buffer)?;
        if header.template_id != TemplateId::PriceTick as u16 {
            return Err(SbeError::UnknownTemplateId(header.template_id));
        }
        Ok(Self { buffer })
    }

    /// UTC timestamp in nanoseconds.
    #[inline(always)]
    pub fn epoch_nanos(&self) -> i64 {
        i64::from_le_bytes(self.buffer[8..16].try_into().unwrap())
    }

    /// Currency pair numeric identifier.
    #[inline(always)]
    pub fn pair_id(&self) -> u32 {
        u32::from_le_bytes(self.buffer[16..20].try_into().unwrap())
    }

    /// Status flags.
    #[inline(always)]
    pub fn flags(&self) -> u32 {
        u32::from_le_bytes(self.buffer[20..24].try_into().unwrap())
    }

    /// Best bid price.
    #[inline(always)]
    pub fn bid(&self) -> f64 {
        f64::from_le_bytes(self.buffer[24..32].try_into().unwrap())
    }

    /// Best ask price.
    #[inline(always)]
    pub fn ask(&self) -> f64 {
        f64::from_le_bytes(self.buffer[32..40].try_into().unwrap())
    }

    /// Calculated mid-market price.
    #[inline(always)]
    pub fn mid(&self) -> f64 {
        (self.bid() + self.ask()) * 0.5
    }
}

/// Encode a PriceTick into destination slice.
#[inline(always)]
pub fn encode_price_tick(tick: &PriceTick, dest: &mut [u8]) -> Result<usize, SbeError> {
    if dest.len() < PRICE_TICK_TOTAL_SIZE {
        return Err(SbeError::BufferTooShort {
            expected: PRICE_TICK_TOTAL_SIZE,
            actual: dest.len(),
        });
    }
    let header = MessageHeader {
        block_length: PRICE_TICK_BLOCK_LENGTH as u16,
        template_id: TemplateId::PriceTick as u16,
        schema_id: CELNET_SBE_SCHEMA_ID,
        version: CELNET_SBE_SCHEMA_VERSION,
    };
    header.encode(dest)?;

    dest[8..16].copy_from_slice(&tick.epoch_nanos.to_le_bytes());
    dest[16..20].copy_from_slice(&tick.pair_id.to_le_bytes());
    dest[20..24].copy_from_slice(&tick.flags.to_le_bytes());
    dest[24..32].copy_from_slice(&tick.bid.to_le_bytes());
    dest[32..40].copy_from_slice(&tick.ask.to_le_bytes());

    Ok(PRICE_TICK_TOTAL_SIZE)
}

// -----------------------------------------------------------------------------
// 2. OptionQuote (Template 102)
// -----------------------------------------------------------------------------
/// Block length for OptionQuote fixed fields (7 * 8B scalar + 112B Greeks = 168B).
pub const OPTION_QUOTE_BLOCK_LENGTH: usize = 168;
/// Total encoded frame length for OptionQuote (8B header + 168B body = 176B).
pub const OPTION_QUOTE_TOTAL_SIZE: usize = SBE_HEADER_SIZE + OPTION_QUOTE_BLOCK_LENGTH;

/// Full two-way option quote with Greek sensitivity strip DTO.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OptionQuote {
    /// Stable quote identifier.
    pub quote_id: u64,
    /// UTC timestamp in nanoseconds since Unix epoch.
    pub epoch_nanos: i64,
    /// Quote validity deadline in nanoseconds (last-look window).
    pub valid_until_nanos: i64,
    /// Bid premium.
    pub bid_price: f64,
    /// Offer / ask premium.
    pub ask_price: f64,
    /// Resolved absolute strike price.
    pub resolved_strike: f64,
    /// Marked vol surface version.
    pub surface_version: u64,
    /// Complete 14-member Greek sensitivities.
    pub greeks: celnet_types::Greeks,
}

/// SBE Flyweight decoder for OptionQuote.
#[derive(Debug)]
pub struct OptionQuoteFlyweight<'a> {
    buffer: &'a [u8],
}

impl<'a> OptionQuoteFlyweight<'a> {
    /// Wrap byte buffer after validating header and frame length.
    #[inline(always)]
    pub fn wrap(buffer: &'a [u8]) -> Result<Self, SbeError> {
        if buffer.len() < OPTION_QUOTE_TOTAL_SIZE {
            return Err(SbeError::BufferTooShort {
                expected: OPTION_QUOTE_TOTAL_SIZE,
                actual: buffer.len(),
            });
        }
        let header = MessageHeader::decode(buffer)?;
        if header.template_id != TemplateId::OptionQuote as u16 {
            return Err(SbeError::UnknownTemplateId(header.template_id));
        }
        Ok(Self { buffer })
    }

    /// Quote identifier.
    #[inline(always)]
    pub fn quote_id(&self) -> u64 {
        u64::from_le_bytes(self.buffer[8..16].try_into().unwrap())
    }

    /// Publication epoch in nanoseconds.
    #[inline(always)]
    pub fn epoch_nanos(&self) -> i64 {
        i64::from_le_bytes(self.buffer[16..24].try_into().unwrap())
    }

    /// Expiration deadline in nanoseconds.
    #[inline(always)]
    pub fn valid_until_nanos(&self) -> i64 {
        i64::from_le_bytes(self.buffer[24..32].try_into().unwrap())
    }

    /// Bid price.
    #[inline(always)]
    pub fn bid_price(&self) -> f64 {
        f64::from_le_bytes(self.buffer[32..40].try_into().unwrap())
    }

    /// Offer / ask price.
    #[inline(always)]
    pub fn ask_price(&self) -> f64 {
        f64::from_le_bytes(self.buffer[40..48].try_into().unwrap())
    }

    /// Resolved strike.
    #[inline(always)]
    pub fn resolved_strike(&self) -> f64 {
        f64::from_le_bytes(self.buffer[48..56].try_into().unwrap())
    }

    /// Surface version.
    #[inline(always)]
    pub fn surface_version(&self) -> u64 {
        u64::from_le_bytes(self.buffer[56..64].try_into().unwrap())
    }

    /// Direct extraction of full 14-member Greek sensitivity strip.
    #[inline(always)]
    pub fn greeks(&self) -> celnet_types::Greeks {
        let b = &self.buffer[64..176];
        celnet_types::Greeks {
            price: f64::from_le_bytes(b[0..8].try_into().unwrap()),
            delta_spot: f64::from_le_bytes(b[8..16].try_into().unwrap()),
            delta_forward: f64::from_le_bytes(b[16..24].try_into().unwrap()),
            gamma: f64::from_le_bytes(b[24..32].try_into().unwrap()),
            vega: f64::from_le_bytes(b[32..40].try_into().unwrap()),
            theta: f64::from_le_bytes(b[40..48].try_into().unwrap()),
            rho_dom: f64::from_le_bytes(b[48..56].try_into().unwrap()),
            rho_for: f64::from_le_bytes(b[56..64].try_into().unwrap()),
            vanna: f64::from_le_bytes(b[64..72].try_into().unwrap()),
            volga: f64::from_le_bytes(b[72..80].try_into().unwrap()),
            charm: f64::from_le_bytes(b[80..88].try_into().unwrap()),
            speed: f64::from_le_bytes(b[88..96].try_into().unwrap()),
            zomma: f64::from_le_bytes(b[96..104].try_into().unwrap()),
            color: f64::from_le_bytes(b[104..112].try_into().unwrap()),
        }
    }
}

/// Encode an OptionQuote into destination slice.
#[inline(always)]
pub fn encode_option_quote(quote: &OptionQuote, dest: &mut [u8]) -> Result<usize, SbeError> {
    if dest.len() < OPTION_QUOTE_TOTAL_SIZE {
        return Err(SbeError::BufferTooShort {
            expected: OPTION_QUOTE_TOTAL_SIZE,
            actual: dest.len(),
        });
    }
    let header = MessageHeader {
        block_length: OPTION_QUOTE_BLOCK_LENGTH as u16,
        template_id: TemplateId::OptionQuote as u16,
        schema_id: CELNET_SBE_SCHEMA_ID,
        version: CELNET_SBE_SCHEMA_VERSION,
    };
    header.encode(dest)?;

    dest[8..16].copy_from_slice(&quote.quote_id.to_le_bytes());
    dest[16..24].copy_from_slice(&quote.epoch_nanos.to_le_bytes());
    dest[24..32].copy_from_slice(&quote.valid_until_nanos.to_le_bytes());
    dest[32..40].copy_from_slice(&quote.bid_price.to_le_bytes());
    dest[40..48].copy_from_slice(&quote.ask_price.to_le_bytes());
    dest[48..56].copy_from_slice(&quote.resolved_strike.to_le_bytes());
    dest[56..64].copy_from_slice(&quote.surface_version.to_le_bytes());

    let g = &quote.greeks;
    let b = &mut dest[64..176];
    b[0..8].copy_from_slice(&g.price.to_le_bytes());
    b[8..16].copy_from_slice(&g.delta_spot.to_le_bytes());
    b[16..24].copy_from_slice(&g.delta_forward.to_le_bytes());
    b[24..32].copy_from_slice(&g.gamma.to_le_bytes());
    b[32..40].copy_from_slice(&g.vega.to_le_bytes());
    b[40..48].copy_from_slice(&g.theta.to_le_bytes());
    b[48..56].copy_from_slice(&g.rho_dom.to_le_bytes());
    b[56..64].copy_from_slice(&g.rho_for.to_le_bytes());
    b[64..72].copy_from_slice(&g.vanna.to_le_bytes());
    b[72..80].copy_from_slice(&g.volga.to_le_bytes());
    b[80..88].copy_from_slice(&g.charm.to_le_bytes());
    b[88..96].copy_from_slice(&g.speed.to_le_bytes());
    b[96..104].copy_from_slice(&g.zomma.to_le_bytes());
    b[104..112].copy_from_slice(&g.color.to_le_bytes());

    Ok(OPTION_QUOTE_TOTAL_SIZE)
}

// -----------------------------------------------------------------------------
// 3. ExecutionReport (Template 104)
// -----------------------------------------------------------------------------
/// Block length for ExecutionReport fixed fields.
pub const EXECUTION_REPORT_BLOCK_LENGTH: usize = 56;
/// Total encoded frame length for ExecutionReport.
pub const EXECUTION_REPORT_TOTAL_SIZE: usize = SBE_HEADER_SIZE + EXECUTION_REPORT_BLOCK_LENGTH;

/// Confirmed trade execution report DTO.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExecutionReport {
    /// Unique execution identifier.
    pub exec_id: u64,
    /// Originating quote identifier.
    pub quote_id: u64,
    /// Execution timestamp in UTC nanoseconds.
    pub epoch_nanos: i64,
    /// Executed unit price.
    pub exec_price: f64,
    /// Executed base notional quantity.
    pub exec_quantity: f64,
    /// Execution side (Buy/Sell).
    pub side: Side,
    /// Execution status.
    pub status: ExecutionStatus,
    /// Liquidity provider numeric identifier.
    pub lp_id: u64,
}

/// SBE Flyweight decoder for ExecutionReport.
#[derive(Debug)]
pub struct ExecutionReportFlyweight<'a> {
    buffer: &'a [u8],
}

impl<'a> ExecutionReportFlyweight<'a> {
    /// Wrap byte buffer after validating header and frame length.
    #[inline(always)]
    pub fn wrap(buffer: &'a [u8]) -> Result<Self, SbeError> {
        if buffer.len() < EXECUTION_REPORT_TOTAL_SIZE {
            return Err(SbeError::BufferTooShort {
                expected: EXECUTION_REPORT_TOTAL_SIZE,
                actual: buffer.len(),
            });
        }
        let header = MessageHeader::decode(buffer)?;
        if header.template_id != TemplateId::ExecutionReport as u16 {
            return Err(SbeError::UnknownTemplateId(header.template_id));
        }
        Ok(Self { buffer })
    }

    /// Execution identifier.
    #[inline(always)]
    pub fn exec_id(&self) -> u64 {
        u64::from_le_bytes(self.buffer[8..16].try_into().unwrap())
    }

    /// Originating quote identifier.
    #[inline(always)]
    pub fn quote_id(&self) -> u64 {
        u64::from_le_bytes(self.buffer[16..24].try_into().unwrap())
    }

    /// Execution timestamp in UTC nanoseconds.
    #[inline(always)]
    pub fn epoch_nanos(&self) -> i64 {
        i64::from_le_bytes(self.buffer[24..32].try_into().unwrap())
    }

    /// Executed price.
    #[inline(always)]
    pub fn exec_price(&self) -> f64 {
        f64::from_le_bytes(self.buffer[32..40].try_into().unwrap())
    }

    /// Executed quantity.
    #[inline(always)]
    pub fn exec_quantity(&self) -> f64 {
        f64::from_le_bytes(self.buffer[40..48].try_into().unwrap())
    }

    /// Trade side.
    #[inline(always)]
    pub fn side(&self) -> Result<Side, SbeError> {
        Side::from_u8(self.buffer[48])
    }

    /// Execution status.
    #[inline(always)]
    pub fn status(&self) -> Result<ExecutionStatus, SbeError> {
        ExecutionStatus::from_u8(self.buffer[49])
    }

    /// LP identifier.
    #[inline(always)]
    pub fn lp_id(&self) -> u64 {
        u64::from_le_bytes(self.buffer[56..64].try_into().unwrap())
    }
}

/// Encode an ExecutionReport into destination slice.
#[inline(always)]
pub fn encode_execution_report(rep: &ExecutionReport, dest: &mut [u8]) -> Result<usize, SbeError> {
    if dest.len() < EXECUTION_REPORT_TOTAL_SIZE {
        return Err(SbeError::BufferTooShort {
            expected: EXECUTION_REPORT_TOTAL_SIZE,
            actual: dest.len(),
        });
    }
    let header = MessageHeader {
        block_length: EXECUTION_REPORT_BLOCK_LENGTH as u16,
        template_id: TemplateId::ExecutionReport as u16,
        schema_id: CELNET_SBE_SCHEMA_ID,
        version: CELNET_SBE_SCHEMA_VERSION,
    };
    header.encode(dest)?;

    dest[8..16].copy_from_slice(&rep.exec_id.to_le_bytes());
    dest[16..24].copy_from_slice(&rep.quote_id.to_le_bytes());
    dest[24..32].copy_from_slice(&rep.epoch_nanos.to_le_bytes());
    dest[32..40].copy_from_slice(&rep.exec_price.to_le_bytes());
    dest[40..48].copy_from_slice(&rep.exec_quantity.to_le_bytes());
    dest[48] = rep.side as u8;
    dest[49] = rep.status as u8;
    dest[50..56].fill(0); // padding
    dest[56..64].copy_from_slice(&rep.lp_id.to_le_bytes());

    Ok(EXECUTION_REPORT_TOTAL_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_tick_round_trip() {
        let tick = PriceTick {
            epoch_nanos: 1_725_450_000_123_456_789,
            pair_id: 1, // EUR/USD
            flags: 0x01,
            bid: 1.0850,
            ask: 1.0852,
        };
        let mut buf = [0u8; PRICE_TICK_TOTAL_SIZE];
        let len = encode_price_tick(&tick, &mut buf).expect("encode succeeds");
        assert_eq!(len, PRICE_TICK_TOTAL_SIZE);

        let fw = PriceTickFlyweight::wrap(&buf).expect("wrap succeeds");
        assert_eq!(fw.epoch_nanos(), tick.epoch_nanos);
        assert_eq!(fw.pair_id(), tick.pair_id);
        assert_eq!(fw.flags(), tick.flags);
        assert_eq!(fw.bid(), tick.bid);
        assert_eq!(fw.ask(), tick.ask);
        assert!((fw.mid() - 1.0851).abs() < 1e-12);
    }

    #[test]
    fn option_quote_round_trip() {
        let quote = OptionQuote {
            quote_id: 99887766,
            epoch_nanos: 1_725_450_000_000_000_000,
            valid_until_nanos: 1_725_450_005_000_000_000,
            bid_price: 0.01234,
            ask_price: 0.01238,
            resolved_strike: 1.0850,
            surface_version: 101,
            greeks: celnet_types::Greeks {
                price: 0.012345,
                delta_spot: 0.4821,
                delta_forward: 0.4933,
                gamma: 2.118,
                vega: 0.305,
                theta: -0.018,
                rho_dom: 0.061,
                rho_for: -0.058,
                vanna: -0.072,
                volga: 0.144,
                charm: 0.0009,
                speed: -1.21,
                zomma: 0.33,
                color: 0.0004,
            },
        };
        let mut buf = [0u8; OPTION_QUOTE_TOTAL_SIZE];
        let len = encode_option_quote(&quote, &mut buf).expect("encode succeeds");
        assert_eq!(len, OPTION_QUOTE_TOTAL_SIZE);

        let fw = OptionQuoteFlyweight::wrap(&buf).expect("wrap succeeds");
        assert_eq!(fw.quote_id(), quote.quote_id);
        assert_eq!(fw.epoch_nanos(), quote.epoch_nanos);
        assert_eq!(fw.valid_until_nanos(), quote.valid_until_nanos);
        assert_eq!(fw.bid_price(), quote.bid_price);
        assert_eq!(fw.ask_price(), quote.ask_price);
        assert_eq!(fw.resolved_strike(), quote.resolved_strike);
        assert_eq!(fw.surface_version(), quote.surface_version);
        assert_eq!(fw.greeks(), quote.greeks);
    }

    #[test]
    fn execution_report_round_trip() {
        let rep = ExecutionReport {
            exec_id: 11223344,
            quote_id: 99887766,
            epoch_nanos: 1_725_450_000_500_000_000,
            exec_price: 0.01236,
            exec_quantity: 10_000_000.0,
            side: Side::Buy,
            status: ExecutionStatus::Filled,
            lp_id: 42,
        };
        let mut buf = [0u8; EXECUTION_REPORT_TOTAL_SIZE];
        let len = encode_execution_report(&rep, &mut buf).expect("encode succeeds");
        assert_eq!(len, EXECUTION_REPORT_TOTAL_SIZE);

        let fw = ExecutionReportFlyweight::wrap(&buf).expect("wrap succeeds");
        assert_eq!(fw.exec_id(), rep.exec_id);
        assert_eq!(fw.quote_id(), rep.quote_id);
        assert_eq!(fw.epoch_nanos(), rep.epoch_nanos);
        assert_eq!(fw.exec_price(), rep.exec_price);
        assert_eq!(fw.exec_quantity(), rep.exec_quantity);
        assert_eq!(fw.side().unwrap(), Side::Buy);
        assert_eq!(fw.status().unwrap(), ExecutionStatus::Filled);
        assert_eq!(fw.lp_id(), 42);
    }

    #[test]
    fn buffer_too_short_is_rejected() {
        let short_buf = [0u8; 10];
        let err = PriceTickFlyweight::wrap(&short_buf).unwrap_err();
        assert!(matches!(err, SbeError::BufferTooShort { .. }));
    }
}
