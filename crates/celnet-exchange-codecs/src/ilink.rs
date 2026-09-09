//! Simple Binary Order Entry Codec (iLink3 compatible SBE).
//!
//! Direct-to-matching-engine binary order entry using Simple Open Framing Headers (SOFH)
//! and fixed-offset SBE structures for NewOrderSingle, OrderCancelReplace,
//! OrderCancelRequest, and ExecutionReport.
#![deny(missing_docs)]

use crate::{ExchangeCodecError, ExchangeSide, ExchangeTimeInForce};

/// Simple Open Framing Header (SOFH) - 4 bytes.
/// Format: 2 bytes message length (inclusive of SOFH), 2 bytes encoding type (0xEB50 for SBE).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sofh {
    /// Total frame length in bytes including the 4-byte SOFH header.
    pub message_length: u16,
    /// Encoding discriminator (0xEB50 standard SBE).
    pub encoding_type: u16,
}

impl Sofh {
    /// Size of SOFH header in bytes.
    pub const SIZE: usize = 4;
    /// Standard SBE encoding type identifier on institutional gateways.
    pub const SBE_ENCODING_TYPE: u16 = 0xEB50;

    /// Decode SOFH header.
    pub fn decode(buf: &[u8]) -> Result<Self, ExchangeCodecError> {
        if buf.len() < Self::SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        let len = u16::from_le_bytes([buf[0], buf[1]]);
        let enc = u16::from_le_bytes([buf[2], buf[3]]);
        if enc != Self::SBE_ENCODING_TYPE {
            return Err(ExchangeCodecError::InvalidFraming {
                expected: Self::SBE_ENCODING_TYPE,
                actual: enc,
            });
        }
        Ok(Self {
            message_length: len,
            encoding_type: enc,
        })
    }

    /// Encode SOFH header.
    pub fn encode(&self, buf: &mut [u8]) -> Result<usize, ExchangeCodecError> {
        if buf.len() < Self::SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        buf[0..2].copy_from_slice(&self.message_length.to_le_bytes());
        buf[2..4].copy_from_slice(&self.encoding_type.to_le_bytes());
        Ok(Self::SIZE)
    }
}

/// SBE message header for order entry messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderMessageHeader {
    /// Root block length.
    pub block_length: u16,
    /// Template ID.
    pub template_id: u16,
    /// Schema ID.
    pub schema_id: u16,
    /// Schema version.
    pub version: u16,
}

impl OrderMessageHeader {
    /// Size of header in bytes.
    pub const SIZE: usize = 8;

    /// Decode header.
    pub fn decode(buf: &[u8]) -> Result<Self, ExchangeCodecError> {
        if buf.len() < Self::SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        let block_length = u16::from_le_bytes([buf[0], buf[1]]);
        let template_id = u16::from_le_bytes([buf[2], buf[3]]);
        let schema_id = u16::from_le_bytes([buf[4], buf[5]]);
        let version = u16::from_le_bytes([buf[6], buf[7]]);
        Ok(Self {
            block_length,
            template_id,
            schema_id,
            version,
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
        buf[0..2].copy_from_slice(&self.block_length.to_le_bytes());
        buf[2..4].copy_from_slice(&self.template_id.to_le_bytes());
        buf[4..6].copy_from_slice(&self.schema_id.to_le_bytes());
        buf[6..8].copy_from_slice(&self.version.to_le_bytes());
        Ok(Self::SIZE)
    }
}

/// New Order Single (Template ID 514).
#[derive(Debug, Clone, PartialEq)]
pub struct NewOrderSingle {
    /// Client order ID (20-byte alphanumeric identifier).
    pub cl_ord_id: [u8; 20],
    /// Exchange security ID.
    pub security_id: u32,
    /// Order side: Buy or Sell.
    pub side: ExchangeSide,
    /// Order quantity.
    pub order_qty: u32,
    /// Order price mantissa.
    pub price_mantissa: i64,
    /// Order price exponent.
    pub price_exponent: i8,
    /// Time in force.
    pub time_in_force: ExchangeTimeInForce,
    /// Manual order indicator (0 = automated, 1 = manual trader).
    pub manual_order_indicator: u8,
    /// Executing trader / desk identifier.
    pub executing_firm_id: [u8; 8],
}

impl NewOrderSingle {
    /// Template ID for NewOrderSingle.
    pub const TEMPLATE_ID: u16 = 514;
    /// Root block length (20 + 4 + 1 + 4 + 8 + 1 + 1 + 1 + 8 = 48 bytes).
    pub const BLOCK_LEN: u16 = 48;
    /// Total wire frame length including SOFH (4) + MessageHeader (8) + Body (48) = 60 bytes.
    pub const WIRE_SIZE: usize = Sofh::SIZE + OrderMessageHeader::SIZE + (Self::BLOCK_LEN as usize);

    /// Compute float price.
    pub fn price_f64(&self) -> f64 {
        (self.price_mantissa as f64) * libm::pow(10.0, self.price_exponent as f64)
    }

    /// Construct from typed parameters.
    pub fn new(
        cl_ord_id_str: &str,
        security_id: u32,
        side: ExchangeSide,
        order_qty: u32,
        price: f64,
        tif: ExchangeTimeInForce,
        manual: bool,
        firm_str: &str,
    ) -> Self {
        let mut cl_ord_id = [b' '; 20];
        let bytes = cl_ord_id_str.as_bytes();
        let copy_len = bytes.len().min(20);
        cl_ord_id[..copy_len].copy_from_slice(&bytes[..copy_len]);

        let mut executing_firm_id = [b' '; 8];
        let fbytes = firm_str.as_bytes();
        let fcopy = fbytes.len().min(8);
        executing_firm_id[..fcopy].copy_from_slice(&fbytes[..fcopy]);

        let price_mantissa = (price * 10_000_000.0).round() as i64;

        Self {
            cl_ord_id,
            security_id,
            side,
            order_qty,
            price_mantissa,
            price_exponent: -7,
            time_in_force: tif,
            manual_order_indicator: if manual { 1 } else { 0 },
            executing_firm_id,
        }
    }

    /// Encode into buffer.
    pub fn encode(&self, buf: &mut [u8]) -> Result<usize, ExchangeCodecError> {
        if buf.len() < Self::WIRE_SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::WIRE_SIZE,
                actual: buf.len(),
            });
        }

        // 1. SOFH
        let sofh = Sofh {
            message_length: Self::WIRE_SIZE as u16,
            encoding_type: Sofh::SBE_ENCODING_TYPE,
        };
        sofh.encode(&mut buf[0..Sofh::SIZE])?;

        // 2. MessageHeader
        let header = OrderMessageHeader {
            block_length: Self::BLOCK_LEN,
            template_id: Self::TEMPLATE_ID,
            schema_id: 1,
            version: 1,
        };
        header.encode(&mut buf[Sofh::SIZE..Sofh::SIZE + OrderMessageHeader::SIZE])?;

        // 3. Body
        let mut offset = Sofh::SIZE + OrderMessageHeader::SIZE;
        buf[offset..offset + 20].copy_from_slice(&self.cl_ord_id);
        offset += 20;
        buf[offset..offset + 4].copy_from_slice(&self.security_id.to_le_bytes());
        offset += 4;
        buf[offset] = self.side.to_u8();
        offset += 1;
        buf[offset..offset + 4].copy_from_slice(&self.order_qty.to_le_bytes());
        offset += 4;
        buf[offset..offset + 8].copy_from_slice(&self.price_mantissa.to_le_bytes());
        offset += 8;
        buf[offset] = self.price_exponent as u8;
        offset += 1;
        buf[offset] = self.time_in_force.to_u8();
        offset += 1;
        buf[offset] = self.manual_order_indicator;
        offset += 1;
        buf[offset..offset + 8].copy_from_slice(&self.executing_firm_id);
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

        let _sofh = Sofh::decode(&buf[0..Sofh::SIZE])?;
        let header = OrderMessageHeader::decode(&buf[Sofh::SIZE..Sofh::SIZE + OrderMessageHeader::SIZE])?;
        if header.template_id != Self::TEMPLATE_ID {
            return Err(ExchangeCodecError::UnknownMessageType(header.template_id));
        }

        let mut offset = Sofh::SIZE + OrderMessageHeader::SIZE;
        let mut cl_ord_id = [0u8; 20];
        cl_ord_id.copy_from_slice(&buf[offset..offset + 20]);
        offset += 20;

        let security_id = u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]]);
        offset += 4;

        let side = ExchangeSide::from_u8(buf[offset])?;
        offset += 1;

        let order_qty = u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]]);
        offset += 4;

        let price_mantissa = i64::from_le_bytes([
            buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3],
            buf[offset + 4], buf[offset + 5], buf[offset + 6], buf[offset + 7],
        ]);
        offset += 8;

        let price_exponent = buf[offset] as i8;
        offset += 1;

        let time_in_force = ExchangeTimeInForce::from_u8(buf[offset])?;
        offset += 1;

        let manual_order_indicator = buf[offset];
        offset += 1;

        let mut executing_firm_id = [0u8; 8];
        executing_firm_id.copy_from_slice(&buf[offset..offset + 8]);

        Ok(Self {
            cl_ord_id,
            security_id,
            side,
            order_qty,
            price_mantissa,
            price_exponent,
            time_in_force,
            manual_order_indicator,
            executing_firm_id,
        })
    }
}

/// Execution Report status tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExecStatus {
    /// Order accepted as new.
    New = b'0',
    /// Order partially filled.
    PartiallyFilled = b'1',
    /// Order completely filled.
    Filled = b'2',
    /// Order cancelled.
    Canceled = b'4',
    /// Order replace confirmed.
    Replaced = b'5',
    /// Order rejected.
    Rejected = b'8',
    /// Order expired.
    Expired = b'C',
}

impl ExecStatus {
    /// Decode from wire byte.
    pub fn from_u8(v: u8) -> Result<Self, ExchangeCodecError> {
        match v {
            b'0' => Ok(Self::New),
            b'1' => Ok(Self::PartiallyFilled),
            b'2' => Ok(Self::Filled),
            b'4' => Ok(Self::Canceled),
            b'5' => Ok(Self::Replaced),
            b'8' => Ok(Self::Rejected),
            b'C' => Ok(Self::Expired),
            other => Err(ExchangeCodecError::InvalidTag(other)),
        }
    }
}

/// Execution Report (Template ID 522).
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionReport {
    /// Client order identifier.
    pub cl_ord_id: [u8; 20],
    /// Exchange order identifier.
    pub order_id: u64,
    /// Unique execution identifier.
    pub exec_id: u64,
    /// Execution status.
    pub status: ExecStatus,
    /// Cumulative executed quantity.
    pub cum_qty: u32,
    /// Remaining leaves quantity.
    pub leaves_qty: u32,
    /// Last fill price mantissa.
    pub last_px_mantissa: i64,
    /// Last fill price exponent.
    pub last_px_exponent: i8,
    /// Last fill quantity.
    pub last_qty: u32,
    /// Transaction timestamp in nanoseconds.
    pub transact_time_nanos: u64,
}

impl ExecutionReport {
    /// Template ID for ExecutionReport.
    pub const TEMPLATE_ID: u16 = 522;
    /// Root block length (20 + 8 + 8 + 1 + 4 + 4 + 8 + 1 + 2 [pad] + 4 + 8 = 68 bytes).
    pub const BLOCK_LEN: u16 = 68;
    /// Total wire frame size.
    pub const WIRE_SIZE: usize = Sofh::SIZE + OrderMessageHeader::SIZE + (Self::BLOCK_LEN as usize);

    /// Compute last fill price as f64.
    pub fn last_px_f64(&self) -> f64 {
        (self.last_px_mantissa as f64) * libm::pow(10.0, self.last_px_exponent as f64)
    }

    /// Encode into buffer.
    pub fn encode(&self, buf: &mut [u8]) -> Result<usize, ExchangeCodecError> {
        if buf.len() < Self::WIRE_SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::WIRE_SIZE,
                actual: buf.len(),
            });
        }

        let sofh = Sofh {
            message_length: Self::WIRE_SIZE as u16,
            encoding_type: Sofh::SBE_ENCODING_TYPE,
        };
        sofh.encode(&mut buf[0..Sofh::SIZE])?;

        let header = OrderMessageHeader {
            block_length: Self::BLOCK_LEN,
            template_id: Self::TEMPLATE_ID,
            schema_id: 1,
            version: 1,
        };
        header.encode(&mut buf[Sofh::SIZE..Sofh::SIZE + OrderMessageHeader::SIZE])?;

        let mut offset = Sofh::SIZE + OrderMessageHeader::SIZE;
        buf[offset..offset + 20].copy_from_slice(&self.cl_ord_id);
        offset += 20;
        buf[offset..offset + 8].copy_from_slice(&self.order_id.to_le_bytes());
        offset += 8;
        buf[offset..offset + 8].copy_from_slice(&self.exec_id.to_le_bytes());
        offset += 8;
        buf[offset] = self.status as u8;
        offset += 1;
        buf[offset..offset + 4].copy_from_slice(&self.cum_qty.to_le_bytes());
        offset += 4;
        buf[offset..offset + 4].copy_from_slice(&self.leaves_qty.to_le_bytes());
        offset += 4;
        buf[offset..offset + 8].copy_from_slice(&self.last_px_mantissa.to_le_bytes());
        offset += 8;
        buf[offset] = self.last_px_exponent as u8;
        offset += 1;
        buf[offset] = 0; // pad
        buf[offset + 1] = 0; // pad
        offset += 2;
        buf[offset..offset + 4].copy_from_slice(&self.last_qty.to_le_bytes());
        offset += 4;
        buf[offset..offset + 8].copy_from_slice(&self.transact_time_nanos.to_le_bytes());
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

        let _sofh = Sofh::decode(&buf[0..Sofh::SIZE])?;
        let header = OrderMessageHeader::decode(&buf[Sofh::SIZE..Sofh::SIZE + OrderMessageHeader::SIZE])?;
        if header.template_id != Self::TEMPLATE_ID {
            return Err(ExchangeCodecError::UnknownMessageType(header.template_id));
        }

        let mut offset = Sofh::SIZE + OrderMessageHeader::SIZE;
        let mut cl_ord_id = [0u8; 20];
        cl_ord_id.copy_from_slice(&buf[offset..offset + 20]);
        offset += 20;

        let order_id = u64::from_le_bytes([
            buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3],
            buf[offset + 4], buf[offset + 5], buf[offset + 6], buf[offset + 7],
        ]);
        offset += 8;

        let exec_id = u64::from_le_bytes([
            buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3],
            buf[offset + 4], buf[offset + 5], buf[offset + 6], buf[offset + 7],
        ]);
        offset += 8;

        let status = ExecStatus::from_u8(buf[offset])?;
        offset += 1;

        let cum_qty = u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]]);
        offset += 4;

        let leaves_qty = u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]]);
        offset += 4;

        let last_px_mantissa = i64::from_le_bytes([
            buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3],
            buf[offset + 4], buf[offset + 5], buf[offset + 6], buf[offset + 7],
        ]);
        offset += 8;

        let last_px_exponent = buf[offset] as i8;
        offset += 3; // exponent + 2 pad

        let last_qty = u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]]);
        offset += 4;

        let transact_time = u64::from_le_bytes([
            buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3],
            buf[offset + 4], buf[offset + 5], buf[offset + 6], buf[offset + 7],
        ]);

        Ok(Self {
            cl_ord_id,
            order_id,
            exec_id,
            status,
            cum_qty,
            leaves_qty,
            last_px_mantissa,
            last_px_exponent,
            last_qty,
            transact_time_nanos: transact_time,
        })
    }
}
