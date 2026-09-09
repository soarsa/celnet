//! Direct Stream Protocol Codec (OUCH-compatible).
//!
//! Compact, fixed-width ASCII/binary streaming protocol optimized for execution venues
//! requiring minimal wire footprint and predictable zero-alloc parsing.
#![deny(missing_docs)]

use crate::{ExchangeCodecError, ExchangeSide, ExchangeTimeInForce};

/// OUCH message types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OuchMessageType {
    /// Inbound: Enter Order ('O').
    EnterOrder = b'O' as isize,
    /// Inbound: Cancel Order ('X').
    CancelOrder = b'X' as isize,
    /// Outbound: Order Accepted ('A').
    OrderAccepted = b'A' as isize,
    /// Outbound: Order Executed ('E').
    OrderExecuted = b'E' as isize,
    /// Outbound: Order Canceled ('C').
    OrderCanceled = b'C' as isize,
    /// Outbound: Order Rejected ('J').
    OrderRejected = b'J' as isize,
}

/// Enter Order packet ('O') - 49 bytes fixed length.
#[derive(Debug, Clone, PartialEq)]
pub struct OuchEnterOrder {
    /// Order token (14 ASCII characters).
    pub order_token: [u8; 14],
    /// Buy or Sell side ('B' or 'S').
    pub side: ExchangeSide,
    /// Quantity of shares / contracts (4 bytes big-endian).
    pub shares: u32,
    /// Security symbol (8 ASCII characters, space-padded).
    pub stock: [u8; 8],
    /// Price in 4-decimal integer fixed-point (e.g. 10000 = $1.0000).
    pub price: u32,
    /// Time in force ('0' = Day, '3' = IOC).
    pub time_in_force: ExchangeTimeInForce,
    /// Firm identifier (4 ASCII characters).
    pub firm: [u8; 4],
}

impl OuchEnterOrder {
    /// Fixed packet size for Enter Order.
    pub const SIZE: usize = 49;
    /// Packet type tag.
    pub const TYPE_TAG: u8 = b'O';

    /// Float price conversion.
    pub fn price_f64(&self) -> f64 {
        (self.price as f64) / 10_000.0
    }

    /// Construct from typed arguments.
    pub fn new(
        token_str: &str,
        side: ExchangeSide,
        shares: u32,
        stock_str: &str,
        price: f64,
        tif: ExchangeTimeInForce,
        firm_str: &str,
    ) -> Self {
        let mut order_token = [b' '; 14];
        let t_bytes = token_str.as_bytes();
        let t_len = t_bytes.len().min(14);
        order_token[..t_len].copy_from_slice(&t_bytes[..t_len]);

        let mut stock = [b' '; 8];
        let s_bytes = stock_str.as_bytes();
        let s_len = s_bytes.len().min(8);
        stock[..s_len].copy_from_slice(&s_bytes[..s_len]);

        let mut firm = [b' '; 4];
        let f_bytes = firm_str.as_bytes();
        let f_len = f_bytes.len().min(4);
        firm[..f_len].copy_from_slice(&f_bytes[..f_len]);

        let price_int = (price * 10_000.0).round() as u32;

        Self {
            order_token,
            side,
            shares,
            stock,
            price: price_int,
            time_in_force: tif,
            firm,
        }
    }

    /// Encode into buffer.
    pub fn encode(&self, buf: &mut [u8]) -> Result<usize, ExchangeCodecError> {
        if buf.len() < Self::SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        buf[0] = Self::TYPE_TAG;
        buf[1..15].copy_from_slice(&self.order_token);
        buf[15] = match self.side {
            ExchangeSide::Buy => b'B',
            ExchangeSide::Sell => b'S',
        };
        buf[16..20].copy_from_slice(&self.shares.to_be_bytes());
        buf[20..28].copy_from_slice(&self.stock);
        buf[28..32].copy_from_slice(&self.price.to_be_bytes());
        buf[32] = match self.time_in_force {
            ExchangeTimeInForce::Day => b'0',
            ExchangeTimeInForce::ImmediateOrCancel => b'3',
            ExchangeTimeInForce::FillOrKill => b'4',
            ExchangeTimeInForce::GoodTillCancel => b'1',
        };
        buf[33..37].copy_from_slice(&self.firm);
        buf[37..49].fill(b' '); // display/capacity padding

        Ok(Self::SIZE)
    }

    /// Decode from buffer.
    pub fn decode(buf: &[u8]) -> Result<Self, ExchangeCodecError> {
        if buf.len() < Self::SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        if buf[0] != Self::TYPE_TAG {
            return Err(ExchangeCodecError::UnknownMessageType(buf[0] as u16));
        }
        let mut order_token = [0u8; 14];
        order_token.copy_from_slice(&buf[1..15]);

        let side = match buf[15] {
            b'B' => ExchangeSide::Buy,
            b'S' => ExchangeSide::Sell,
            other => return Err(ExchangeCodecError::InvalidTag(other)),
        };

        let shares = u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]);
        let mut stock = [0u8; 8];
        stock.copy_from_slice(&buf[20..28]);

        let price = u32::from_be_bytes([buf[28], buf[29], buf[30], buf[31]]);

        let time_in_force = match buf[32] {
            b'0' => ExchangeTimeInForce::Day,
            b'3' => ExchangeTimeInForce::ImmediateOrCancel,
            b'4' => ExchangeTimeInForce::FillOrKill,
            b'1' => ExchangeTimeInForce::GoodTillCancel,
            other => return Err(ExchangeCodecError::InvalidTag(other)),
        };

        let mut firm = [0u8; 4];
        firm.copy_from_slice(&buf[33..37]);

        Ok(Self {
            order_token,
            side,
            shares,
            stock,
            price,
            time_in_force,
            firm,
        })
    }
}

/// Order Executed packet ('E') - 40 bytes fixed length.
#[derive(Debug, Clone, PartialEq)]
pub struct OuchOrderExecuted {
    /// Timestamp nanoseconds since midnight.
    pub timestamp_nanos: u64,
    /// Client order token.
    pub order_token: [u8; 14],
    /// Executed shares / contracts.
    pub executed_shares: u32,
    /// Execution price in 4-decimal fixed point.
    pub execution_price: u32,
    /// Exchange match number.
    pub match_number: u64,
}

impl OuchOrderExecuted {
    /// Size of Order Executed packet.
    pub const SIZE: usize = 40;
    /// Message tag.
    pub const TYPE_TAG: u8 = b'E';

    /// Float execution price.
    pub fn execution_price_f64(&self) -> f64 {
        (self.execution_price as f64) / 10_000.0
    }

    /// Encode into buffer.
    pub fn encode(&self, buf: &mut [u8]) -> Result<usize, ExchangeCodecError> {
        if buf.len() < Self::SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        buf[0] = Self::TYPE_TAG;
        buf[1..9].copy_from_slice(&self.timestamp_nanos.to_be_bytes());
        buf[9..23].copy_from_slice(&self.order_token);
        buf[23..27].copy_from_slice(&self.executed_shares.to_be_bytes());
        buf[27..31].copy_from_slice(&self.execution_price.to_be_bytes());
        buf[31..39].copy_from_slice(&self.match_number.to_be_bytes());
        buf[39] = b'Y'; // liquidation flag
        Ok(Self::SIZE)
    }

    /// Decode from buffer.
    pub fn decode(buf: &[u8]) -> Result<Self, ExchangeCodecError> {
        if buf.len() < Self::SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        if buf[0] != Self::TYPE_TAG {
            return Err(ExchangeCodecError::UnknownMessageType(buf[0] as u16));
        }

        let timestamp_nanos = u64::from_be_bytes([
            buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8],
        ]);
        let mut order_token = [0u8; 14];
        order_token.copy_from_slice(&buf[9..23]);

        let executed_shares = u32::from_be_bytes([buf[23], buf[24], buf[25], buf[26]]);
        let execution_price = u32::from_be_bytes([buf[27], buf[28], buf[29], buf[30]]);
        let match_number = u64::from_be_bytes([
            buf[31], buf[32], buf[33], buf[34], buf[35], buf[36], buf[37], buf[38],
        ]);

        Ok(Self {
            timestamp_nanos,
            order_token,
            executed_shares,
            execution_price,
            match_number,
        })
    }
}
