//! `celnet-exchange-codecs` — Native Exchange Binary Protocol Codecs.
//!
//! Provides zero-allocation, byte-aligned, endian-safe serialization and flyweight decoders
//! for global exchange matching engine binary protocols:
//!
//! 1. **Simple Binary Market Data (MDP-compatible)**: Multicast incremental book refresh,
//!    snapshot, and trade summaries using Simple Binary Encoding (SBE).
//! 2. **Simple Binary Order Entry (iLink-compatible)**: Simple Open Framing Header (SOFH) +
//!    SBE order entry (NewOrderSingle, OrderCancelReplace, ExecutionReport).
//! 3. **Binary Framed Protocol (ETI-compatible)**: High-throughput fixed-frame binary codec.
//! 4. **Direct Stream Protocol (OUCH-compatible)**: Byte-aligned fixed-width binary stream.
//! 5. **Transcoder**: Zero-alloc conversion bridging native exchange wire frames into
//!    Celnet domain models (`celnet-types` and `celnet-sbe`).
//!
//! All codecs adhere to `#![forbid(unsafe_code)]` and zero-alloc hot-path design.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod eti;
pub mod gateway;
pub mod ilink;
pub mod mdp;
pub mod ouch;
pub mod transcoder;

use thiserror::Error;

/// Error conditions encountered during exchange binary decoding or encoding.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum ExchangeCodecError {
    /// Provided buffer is smaller than the required frame size.
    #[error("buffer underflow: expected at least {expected} bytes, got {actual}")]
    BufferUnderflow {
        /// Required minimum bytes.
        expected: usize,
        /// Actual bytes present in buffer.
        actual: usize,
    },
    /// Invalid magic byte or framing discriminator.
    #[error("invalid framing identifier: expected 0x{expected:04x}, got 0x{actual:04x}")]
    InvalidFraming {
        /// Expected identifier.
        expected: u16,
        /// Actual identifier parsed.
        actual: u16,
    },
    /// Unknown template or message type discriminator.
    #[error("unknown message type discriminator: {0}")]
    UnknownMessageType(u16),
    /// Invalid field or enum tag on the wire.
    #[error("invalid enum tag value: {0}")]
    InvalidTag(u8),
    /// Schema or version mismatch.
    #[error("version mismatch: expected {expected}, got {actual}")]
    VersionMismatch {
        /// Expected version.
        expected: u16,
        /// Actual version.
        actual: u16,
    },
    /// String conversion error (e.g. invalid ASCII or non-UTF8 payload).
    #[error("invalid text payload encoding")]
    InvalidEncoding,
}

/// Standardized exchange trading side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum ExchangeSide {
    /// Buy / Bid side.
    Buy = 1,
    /// Sell / Offer side.
    Sell = 2,
}

impl ExchangeSide {
    /// Convert byte to ExchangeSide.
    pub fn from_u8(val: u8) -> Result<Self, ExchangeCodecError> {
        match val {
            1 | b'1' | b'B' => Ok(Self::Buy),
            2 | b'2' | b'S' => Ok(Self::Sell),
            other => Err(ExchangeCodecError::InvalidTag(other)),
        }
    }

    /// Wire byte representation.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Buy => 1,
            Self::Sell => 2,
        }
    }
}

/// Standardized time in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum ExchangeTimeInForce {
    /// Good for Day.
    Day = 0,
    /// Good 'Til Cancelled.
    GoodTillCancel = 1,
    /// Immediate or Cancel.
    ImmediateOrCancel = 3,
    /// Fill or Kill.
    FillOrKill = 4,
}

impl ExchangeTimeInForce {
    /// Convert byte to ExchangeTimeInForce.
    pub fn from_u8(val: u8) -> Result<Self, ExchangeCodecError> {
        match val {
            0 | b'0' => Ok(Self::Day),
            1 | b'1' => Ok(Self::GoodTillCancel),
            3 | b'3' | b'I' => Ok(Self::ImmediateOrCancel),
            4 | b'4' | b'F' => Ok(Self::FillOrKill),
            other => Err(ExchangeCodecError::InvalidTag(other)),
        }
    }

    /// Wire byte representation.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Day => 0,
            Self::GoodTillCancel => 1,
            Self::ImmediateOrCancel => 3,
            Self::FillOrKill => 4,
        }
    }
}

pub use gateway::{
    BookAction, BookUpdateDispatcher, FeedHealth, GapAction, MarketDataFeedHandler,
};

