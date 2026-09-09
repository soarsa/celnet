//! The cross-asset (Equities, Commodities, Crypto) options dialect mapping.
//!
//! Extends Celnet FIX capabilities from FX and linear rates to Equities,
//! Commodities, and Digital Assets (Crypto), conforming to FIX 5.0 SP2 and
//! FIX Latest standards.
//!
//! Follows the zero-copy, wire-specified discipline of [`crate::dialect_fx`]
//! and [`crate::dialect_rates`].

#![forbid(unsafe_code)]

use celnet_types::OptionType;

use crate::dialect_fx::{
    EXERCISE_AMERICAN, TAG_EXPIRY_YEARS, parse_float, parse_put_or_call,
};
use crate::dictionary::MsgType;
use crate::framing::{FrameCursor, FrameEncoder};
use crate::messages::Header;

/// FIX `Product(460)` for Commodity.
pub const PRODUCT_COMMODITY: u32 = 2;
/// FIX `Product(460)` for Equity.
pub const PRODUCT_EQUITY: u32 = 7;
/// FIX `Product(460)` for Digital Asset / Crypto.
pub const PRODUCT_DIGITAL_ASSET: u32 = 12;

/// FIX `SecurityType(167)` for standard options.
pub const SEC_TYPE_OPT: &[u8] = b"OPT";

/// Asset family of the cross-asset contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossAssetProductKind {
    /// Equity single-name or index option.
    Equity,
    /// Commodity energy/metals/ag option.
    Commodity,
    /// Crypto / digital asset option.
    DigitalAsset,
}

impl CrossAssetProductKind {
    /// Map from FIX `Product(460)` tag value.
    pub fn from_product_id(id: u32) -> Option<Self> {
        match id {
            PRODUCT_EQUITY => Some(Self::Equity),
            PRODUCT_COMMODITY => Some(Self::Commodity),
            PRODUCT_DIGITAL_ASSET => Some(Self::DigitalAsset),
            _ => None,
        }
    }

    /// Convert to FIX `Product(460)` tag value.
    #[must_use]
    pub fn to_product_id(self) -> u32 {
        match self {
            Self::Equity => PRODUCT_EQUITY,
            Self::Commodity => PRODUCT_COMMODITY,
            Self::DigitalAsset => PRODUCT_DIGITAL_ASSET,
        }
    }
}

/// Client intent on a cross-asset option quote request.
#[derive(Debug, Clone, PartialEq)]
pub struct CrossAssetOptionRfq {
    /// Inbound `QuoteReqID(131)`.
    pub quote_req_id: String,
    /// Product family (`Product(460)`).
    pub product_kind: CrossAssetProductKind,
    /// Underlying symbol (`Symbol(55)`).
    pub symbol: String,
    /// Call or Put (`PutOrCall(201)`).
    pub option_type: OptionType,
    /// Strike price (`StrikePrice(44)` or `(202)`).
    pub strike: f64,
    /// Time to expiration in years (`TAG_EXPIRY_YEARS(7001)`).
    pub expiry_years: f64,
    /// Requested notional / quantity (`OrderQty(38)`).
    pub notional: f64,
    /// Side (`Side(54)`), or None if two-way.
    pub side: Option<u8>,
    /// Exercise style: European (0) or American (1).
    pub is_american: bool,
}

/// Outbound quote response for a cross-asset option.
#[derive(Debug, Clone, PartialEq)]
pub struct CrossAssetQuoteOut {
    /// Inbound `QuoteReqID(131)`.
    pub quote_req_id: String,
    /// Generated unique `QuoteID(117)`.
    pub quote_id: String,
    /// Underlying symbol (`Symbol(55)`).
    pub symbol: String,
    /// Two-way bid price.
    pub bid_price: Option<f64>,
    /// Two-way offer/ask price.
    pub ask_price: Option<f64>,
    /// Available bid size.
    pub bid_size: Option<f64>,
    /// Available offer size.
    pub ask_size: Option<f64>,
}

/// Errors occurring during cross-asset dialect decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossAssetDialectError {
    /// Missing `QuoteReqID(131)`.
    MissingQuoteReqId,
    /// Missing or unrecognized `Product(460)`.
    BadProduct,
    /// Missing `Symbol(55)`.
    MissingSymbol,
    /// Missing or invalid `PutOrCall(201)`.
    BadPutOrCall,
    /// Missing or non-positive `StrikePrice(44)`/`(202)`.
    BadStrike,
    /// Missing or non-positive `TAG_EXPIRY_YEARS(7001)`.
    BadExpiry,
    /// Missing or non-positive `OrderQty(38)`.
    BadNotional,
}

/// Decode an inbound cross-asset `QuoteRequest(35=R)`.
pub fn decode_cross_asset_rfq(cursor: &FrameCursor<'_>) -> Result<CrossAssetOptionRfq, CrossAssetDialectError> {
    let quote_req_id = cursor
        .get(131)
        .and_then(|v| std::str::from_utf8(v).ok())
        .map(String::from)
        .ok_or(CrossAssetDialectError::MissingQuoteReqId)?;

    let prod_id = cursor
        .get(460)
        .and_then(|v| std::str::from_utf8(v).ok())
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or(CrossAssetDialectError::BadProduct)?;
    let product_kind = CrossAssetProductKind::from_product_id(prod_id)
        .ok_or(CrossAssetDialectError::BadProduct)?;

    let symbol = cursor
        .get(55)
        .and_then(|v| std::str::from_utf8(v).ok())
        .map(String::from)
        .ok_or(CrossAssetDialectError::MissingSymbol)?;

    let option_type = cursor
        .get(201)
        .and_then(parse_put_or_call)
        .ok_or(CrossAssetDialectError::BadPutOrCall)?;

    let strike = cursor
        .get(44)
        .or_else(|| cursor.get(202))
        .and_then(parse_float)
        .filter(|&s| s > 0.0)
        .ok_or(CrossAssetDialectError::BadStrike)?;

    let expiry_years = cursor
        .get(TAG_EXPIRY_YEARS)
        .and_then(parse_float)
        .filter(|&t| t > 0.0)
        .ok_or(CrossAssetDialectError::BadExpiry)?;

    let notional = cursor
        .get(38)
        .and_then(parse_float)
        .filter(|&n| n > 0.0)
        .ok_or(CrossAssetDialectError::BadNotional)?;

    let side = cursor.get(54).and_then(|v| v.first().copied());
    let is_american = cursor
        .get(1194)
        .and_then(|v| std::str::from_utf8(v).ok())
        .and_then(|s| s.parse::<u32>().ok())
        == Some(EXERCISE_AMERICAN);

    Ok(CrossAssetOptionRfq {
        quote_req_id,
        product_kind,
        symbol,
        option_type,
        strike,
        expiry_years,
        notional,
        side,
        is_american,
    })
}

/// Encode an outbound `Quote(35=S)` response for cross-asset pricing.
pub fn encode_cross_asset_quote(
    header: &Header<'_>,
    quote: &CrossAssetQuoteOut,
    encoder: &mut FrameEncoder,
) -> Vec<u8> {
    encoder.clear();
    header.encode(MsgType::Quote, encoder);
    encoder.push(131, quote.quote_req_id.as_bytes());
    encoder.push(117, quote.quote_id.as_bytes());
    encoder.push(55, quote.symbol.as_bytes());

    if let Some(bid) = quote.bid_price {
        let s = format!("{:.6}", bid);
        encoder.push(132, s.as_bytes());
    }
    if let Some(ask) = quote.ask_price {
        let s = format!("{:.6}", ask);
        encoder.push(133, s.as_bytes());
    }
    if let Some(bsz) = quote.bid_size {
        let s = format!("{:.2}", bsz);
        encoder.push(134, s.as_bytes());
    }
    if let Some(asz) = quote.ask_size {
        let s = format!("{:.2}", asz);
        encoder.push(135, s.as_bytes());
    }
    encoder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_asset_equity_option_round_trip() {
        let mut enc = FrameEncoder::new();
        let hdr = Header {
            sender: b"CLIENT",
            target: b"CELNET",
            seq_num: 1,
            sending_time: b"20260904-22:00:00.000",
        };

        hdr.encode(MsgType::QuoteRequest, &mut enc);
        enc.push(131, b"RFQ-EQ-001");
        enc.push(460, b"7"); // PRODUCT_EQUITY
        enc.push(55, b"AAPL");
        enc.push(201, b"1"); // CALL
        enc.push(44, b"250.0");
        enc.push(TAG_EXPIRY_YEARS, b"0.5");
        enc.push(38, b"10000.0");
        let raw = enc.finish();

        let cursor = FrameCursor::parse(&raw).expect("valid frame");
        let rfq = decode_cross_asset_rfq(&cursor).expect("decodes clean");

        assert_eq!(rfq.quote_req_id, "RFQ-EQ-001");
        assert_eq!(rfq.product_kind, CrossAssetProductKind::Equity);
        assert_eq!(rfq.symbol, "AAPL");
        assert_eq!(rfq.option_type, OptionType::Call);
        assert_eq!(rfq.strike, 250.0);
        assert_eq!(rfq.expiry_years, 0.5);
        assert_eq!(rfq.notional, 10000.0);
        assert!(!rfq.is_american);
    }

    #[test]
    fn cross_asset_crypto_option_round_trip() {
        let mut enc = FrameEncoder::new();
        let hdr = Header {
            sender: b"CLIENT",
            target: b"CELNET",
            seq_num: 2,
            sending_time: b"20260904-22:00:00.000",
        };

        hdr.encode(MsgType::QuoteRequest, &mut enc);
        enc.push(131, b"RFQ-BTC-999");
        enc.push(460, b"12"); // PRODUCT_DIGITAL_ASSET
        enc.push(55, b"BTCUSD");
        enc.push(201, b"0"); // PUT
        enc.push(44, b"95000.0");
        enc.push(TAG_EXPIRY_YEARS, b"0.25");
        enc.push(38, b"5.0");
        let raw = enc.finish();

        let cursor = FrameCursor::parse(&raw).expect("valid frame");
        let rfq = decode_cross_asset_rfq(&cursor).expect("decodes clean");

        assert_eq!(rfq.quote_req_id, "RFQ-BTC-999");
        assert_eq!(rfq.product_kind, CrossAssetProductKind::DigitalAsset);
        assert_eq!(rfq.symbol, "BTCUSD");
        assert_eq!(rfq.option_type, OptionType::Put);
        assert_eq!(rfq.strike, 95000.0);
        assert_eq!(rfq.expiry_years, 0.25);
    }

    #[test]
    fn cross_asset_quote_encode() {
        let mut enc = FrameEncoder::new();
        let hdr = Header {
            sender: b"CELNET",
            target: b"CLIENT",
            seq_num: 3,
            sending_time: b"20260904-22:00:01.000",
        };

        let quote = CrossAssetQuoteOut {
            quote_req_id: "RFQ-EQ-001".to_string(),
            quote_id: "QUOTE-EQ-777".to_string(),
            symbol: "AAPL".to_string(),
            bid_price: Some(12.50),
            ask_price: Some(12.75),
            bid_size: Some(100.0),
            ask_size: Some(100.0),
        };

        let raw = encode_cross_asset_quote(&hdr, &quote, &mut enc);
        let cursor = FrameCursor::parse(&raw).expect("valid quote frame");
        assert_eq!(cursor.get(131), Some(b"RFQ-EQ-001".as_slice()));
        assert_eq!(cursor.get(117), Some(b"QUOTE-EQ-777".as_slice()));
        assert_eq!(cursor.get(55), Some(b"AAPL".as_slice()));
        assert_eq!(cursor.get(132), Some(b"12.500000".as_slice()));
        assert_eq!(cursor.get(133), Some(b"12.750000".as_slice()));
    }
}
