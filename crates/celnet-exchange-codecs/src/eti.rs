//! Binary Framed Protocol Codec (ETI compatible).
//!
//! Ultra-high throughput binary framed protocol used on European derivatives exchanges,
//! featuring 8-byte framing headers with message lengths and template IDs.
#![deny(missing_docs)]

use crate::{ExchangeCodecError, ExchangeSide, ExchangeTimeInForce};

/// Frame Header (8 bytes): 4 bytes body length, 2 bytes template id, 2 bytes interface id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Length of message payload in bytes (excluding this 8-byte header).
    pub body_len: u32,
    /// Message template discriminator.
    pub template_id: u16,
    /// Interface identifier (e.g. 100 for Order Entry, 200 for Market Data).
    pub interface_id: u16,
}

impl FrameHeader {
    /// Size of frame header in bytes.
    pub const SIZE: usize = 8;

    /// Decode header.
    pub fn decode(buf: &[u8]) -> Result<Self, ExchangeCodecError> {
        if buf.len() < Self::SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        let body_len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let template_id = u16::from_le_bytes([buf[4], buf[5]]);
        let interface_id = u16::from_le_bytes([buf[6], buf[7]]);
        Ok(Self {
            body_len,
            template_id,
            interface_id,
        })
    }

    /// Encode header.
    pub fn encode(&self, buf: &mut [u8]) -> Result<usize, ExchangeCodecError> {
        if buf.len() < Self::SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.body_len.to_le_bytes());
        buf[4..6].copy_from_slice(&self.template_id.to_le_bytes());
        buf[6..8].copy_from_slice(&self.interface_id.to_le_bytes());
        Ok(Self::SIZE)
    }
}

/// Binary Framed Order Request (Template ID 10100).
#[derive(Debug, Clone, PartialEq)]
pub struct FramedOrderRequest {
    /// Client order ID (16 bytes).
    pub cl_ord_id: u64,
    /// Numeric instrument identifier.
    pub security_id: u64,
    /// Trading side.
    pub side: ExchangeSide,
    /// Time in force.
    pub time_in_force: ExchangeTimeInForce,
    /// Order quantity.
    pub order_qty: u64,
    /// Limit price in 8-decimal fixed-point units.
    pub price_scaled: i64,
}

impl FramedOrderRequest {
    /// Template ID for framed order.
    pub const TEMPLATE_ID: u16 = 10100;
    /// Body length in bytes (8 + 8 + 1 + 1 + 6 [pad] + 8 + 8 = 40).
    pub const BODY_LEN: u32 = 40;
    /// Total wire size with 8-byte header = 48 bytes.
    pub const WIRE_SIZE: usize = FrameHeader::SIZE + (Self::BODY_LEN as usize);

    /// Convert price to f64.
    pub fn price_f64(&self) -> f64 {
        (self.price_scaled as f64) / 100_000_000.0
    }

    /// Encode into buffer.
    pub fn encode(&self, buf: &mut [u8]) -> Result<usize, ExchangeCodecError> {
        if buf.len() < Self::WIRE_SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::WIRE_SIZE,
                actual: buf.len(),
            });
        }

        let header = FrameHeader {
            body_len: Self::BODY_LEN,
            template_id: Self::TEMPLATE_ID,
            interface_id: 100,
        };
        header.encode(&mut buf[0..FrameHeader::SIZE])?;

        let mut offset = FrameHeader::SIZE;
        buf[offset..offset + 8].copy_from_slice(&self.cl_ord_id.to_le_bytes());
        offset += 8;
        buf[offset..offset + 8].copy_from_slice(&self.security_id.to_le_bytes());
        offset += 8;
        buf[offset] = self.side.to_u8();
        offset += 1;
        buf[offset] = self.time_in_force.to_u8();
        offset += 1;
        buf[offset..offset + 6].fill(0); // padding
        offset += 6;
        buf[offset..offset + 8].copy_from_slice(&self.order_qty.to_le_bytes());
        offset += 8;
        buf[offset..offset + 8].copy_from_slice(&self.price_scaled.to_le_bytes());
        offset += 8;

        Ok(offset)
    }

    /// Decode from buffer.
    pub fn decode(buf: &[u8]) -> Result<Self, ExchangeCodecError> {
        if buf.len() < Self::WIRE_SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::WIRE_SIZE,
                actual: buf.len(),
            });
        }

        let header = FrameHeader::decode(&buf[0..FrameHeader::SIZE])?;
        if header.template_id != Self::TEMPLATE_ID {
            return Err(ExchangeCodecError::UnknownMessageType(header.template_id));
        }

        let mut offset = FrameHeader::SIZE;
        let cl_ord_id = u64::from_le_bytes([
            buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3],
            buf[offset + 4], buf[offset + 5], buf[offset + 6], buf[offset + 7],
        ]);
        offset += 8;

        let security_id = u64::from_le_bytes([
            buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3],
            buf[offset + 4], buf[offset + 5], buf[offset + 6], buf[offset + 7],
        ]);
        offset += 8;

        let side = ExchangeSide::from_u8(buf[offset])?;
        offset += 1;

        let time_in_force = ExchangeTimeInForce::from_u8(buf[offset])?;
        offset += 1 + 6; // skip pad

        let order_qty = u64::from_le_bytes([
            buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3],
            buf[offset + 4], buf[offset + 5], buf[offset + 6], buf[offset + 7],
        ]);
        offset += 8;

        let price_scaled = i64::from_le_bytes([
            buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3],
            buf[offset + 4], buf[offset + 5], buf[offset + 6], buf[offset + 7],
        ]);

        Ok(Self {
            cl_ord_id,
            security_id,
            side,
            time_in_force,
            order_qty,
            price_scaled,
        })
    }
}
