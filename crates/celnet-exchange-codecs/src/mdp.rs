//! Simple Binary Market Data Codec (MDP 3.0 compatible SBE).
//!
//! High-throughput multicast market data protocol implementing channel packet headers,
//! message headers, incremental book depth updates, and trade summary frames.
#![deny(missing_docs)]

use crate::ExchangeCodecError;

/// Binary packet header prepended to UDP multicast datagrams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketHeader {
    /// Sequence number of the first message in the packet.
    pub sequence_number: u32,
    /// Sending timestamp in nanoseconds since UNIX epoch.
    pub sending_time_nanos: u64,
}

impl PacketHeader {
    /// Size of the packet header in bytes (4 + 8 = 12).
    pub const SIZE: usize = 12;

    /// Decode packet header from buffer.
    pub fn decode(buf: &[u8]) -> Result<Self, ExchangeCodecError> {
        if buf.len() < Self::SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        let seq = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let time = u64::from_le_bytes([
            buf[4], buf[5], buf[6], buf[7], buf[8], buf[9], buf[10], buf[11],
        ]);
        Ok(Self {
            sequence_number: seq,
            sending_time_nanos: time,
        })
    }

    /// Encode packet header into buffer.
    pub fn encode(&self, buf: &mut [u8]) -> Result<usize, ExchangeCodecError> {
        if buf.len() < Self::SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.sequence_number.to_le_bytes());
        buf[4..12].copy_from_slice(&self.sending_time_nanos.to_le_bytes());
        Ok(Self::SIZE)
    }
}

/// SBE message header (8 bytes: block_length, template_id, schema_id, version).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageHeader {
    /// Block length of root message fields.
    pub block_length: u16,
    /// Template identifier.
    pub template_id: u16,
    /// Schema identifier.
    pub schema_id: u16,
    /// Schema version.
    pub version: u16,
}

impl MessageHeader {
    /// Length of SBE message header in bytes.
    pub const SIZE: usize = 8;

    /// Decode SBE message header.
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

    /// Encode SBE message header.
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

/// Market data update action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UpdateAction {
    /// New level inserted into book.
    New = 0,
    /// Existing level changed in price or quantity.
    Change = 1,
    /// Existing level deleted from book.
    Delete = 2,
    /// Overlay existing level.
    Overlay = 5,
}

impl UpdateAction {
    /// Convert byte to UpdateAction.
    pub fn from_u8(v: u8) -> Result<Self, ExchangeCodecError> {
        match v {
            0 => Ok(Self::New),
            1 => Ok(Self::Change),
            2 => Ok(Self::Delete),
            5 => Ok(Self::Overlay),
            other => Err(ExchangeCodecError::InvalidTag(other)),
        }
    }
}

/// Market data entry type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EntryType {
    /// Bid book level.
    Bid = b'0',
    /// Offer / ask book level.
    Offer = b'1',
    /// Trade print summary.
    Trade = b'2',
    /// Implied bid level.
    ImpliedBid = b'E',
    /// Implied offer level.
    ImpliedOffer = b'F',
    /// Empty book reset.
    BookReset = b'J',
}

impl EntryType {
    /// Convert byte to EntryType.
    pub fn from_u8(v: u8) -> Result<Self, ExchangeCodecError> {
        match v {
            b'0' => Ok(Self::Bid),
            b'1' => Ok(Self::Offer),
            b'2' => Ok(Self::Trade),
            b'E' => Ok(Self::ImpliedBid),
            b'F' => Ok(Self::ImpliedOffer),
            b'J' => Ok(Self::BookReset),
            other => Err(ExchangeCodecError::InvalidTag(other)),
        }
    }
}

/// A single price level or trade record in an incremental refresh message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BookEntry {
    /// Action: New, Change, Delete.
    pub action: UpdateAction,
    /// Entry type: Bid, Offer, Trade.
    pub entry_type: EntryType,
    /// Security numeric identifier.
    pub security_id: u32,
    /// Sequence number of update for this security.
    pub repeat_seq: u32,
    /// Price mantissa (fixed-point integer).
    pub price_mantissa: i64,
    /// Price exponent (e.g. -7 for 10^-7).
    pub price_exponent: i8,
    /// Quantity / size at this level.
    pub size: u32,
    /// Number of discrete orders comprising this level.
    pub number_of_orders: u32,
}

impl BookEntry {
    /// Size of a single book entry on the wire (1 + 1 + 2 [pad] + 4 + 4 + 8 + 1 + 3 [pad] + 4 + 4 = 32 bytes).
    pub const SIZE: usize = 32;

    /// Calculate real floating-point price.
    pub fn price_f64(&self) -> f64 {
        (self.price_mantissa as f64) * libm::pow(10.0, self.price_exponent as f64)
    }

    /// Construct from float price and size.
    pub fn from_price_and_size(
        action: UpdateAction,
        entry_type: EntryType,
        security_id: u32,
        repeat_seq: u32,
        price: f64,
        size: u32,
        number_of_orders: u32,
    ) -> Self {
        let mantissa = (price * 10_000_000.0).round() as i64;
        Self {
            action,
            entry_type,
            security_id,
            repeat_seq,
            price_mantissa: mantissa,
            price_exponent: -7,
            size,
            number_of_orders,
        }
    }

    /// Decode entry from slice.
    pub fn decode(buf: &[u8]) -> Result<Self, ExchangeCodecError> {
        if buf.len() < Self::SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        let action = UpdateAction::from_u8(buf[0])?;
        let entry_type = EntryType::from_u8(buf[1])?;
        let security_id = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let repeat_seq = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let price_mantissa = i64::from_le_bytes([
            buf[12], buf[13], buf[14], buf[15], buf[16], buf[17], buf[18], buf[19],
        ]);
        let price_exponent = buf[20] as i8;
        let size = u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]);
        let number_of_orders = u32::from_le_bytes([buf[28], buf[29], buf[30], buf[31]]);

        Ok(Self {
            action,
            entry_type,
            security_id,
            repeat_seq,
            price_mantissa,
            price_exponent,
            size,
            number_of_orders,
        })
    }

    /// Encode entry into slice.
    pub fn encode(&self, buf: &mut [u8]) -> Result<usize, ExchangeCodecError> {
        if buf.len() < Self::SIZE {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        buf[0] = self.action as u8;
        buf[1] = self.entry_type as u8;
        buf[2] = 0;
        buf[3] = 0;
        buf[4..8].copy_from_slice(&self.security_id.to_le_bytes());
        buf[8..12].copy_from_slice(&self.repeat_seq.to_le_bytes());
        buf[12..20].copy_from_slice(&self.price_mantissa.to_le_bytes());
        buf[20] = self.price_exponent as u8;
        buf[21] = 0;
        buf[22] = 0;
        buf[23] = 0;
        buf[24..28].copy_from_slice(&self.size.to_le_bytes());
        buf[28..32].copy_from_slice(&self.number_of_orders.to_le_bytes());
        Ok(Self::SIZE)
    }
}

/// Market Data Incremental Refresh message (Template ID 46).
#[derive(Debug, Clone, PartialEq)]
pub struct IncrementalRefresh {
    /// Transaction time in nanoseconds.
    pub transact_time_nanos: u64,
    /// Bitmask indicator for end of packet or event.
    pub match_event_indicator: u8,
    /// Repeating group entries.
    pub entries: Vec<BookEntry>,
}

impl IncrementalRefresh {
    /// Template ID for IncrementalRefresh.
    pub const TEMPLATE_ID: u16 = 46;
    /// Schema ID.
    pub const SCHEMA_ID: u16 = 1;
    /// Schema Version.
    pub const VERSION: u16 = 1;
    /// Root block length (transact_time 8 + match_event_indicator 1 + pad 1 + num_in_group 2 = 12).
    pub const ROOT_BLOCK_LEN: u16 = 12;

    /// Total wire length including headers.
    pub fn wire_size(&self) -> usize {
        MessageHeader::SIZE + (Self::ROOT_BLOCK_LEN as usize) + self.entries.len() * BookEntry::SIZE
    }

    /// Encode into buffer.
    pub fn encode(&self, buf: &mut [u8]) -> Result<usize, ExchangeCodecError> {
        let total = self.wire_size();
        if buf.len() < total {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: total,
                actual: buf.len(),
            });
        }

        let header = MessageHeader {
            block_length: Self::ROOT_BLOCK_LEN,
            template_id: Self::TEMPLATE_ID,
            schema_id: Self::SCHEMA_ID,
            version: Self::VERSION,
        };
        header.encode(&mut buf[0..MessageHeader::SIZE])?;

        let mut offset = MessageHeader::SIZE;
        buf[offset..offset + 8].copy_from_slice(&self.transact_time_nanos.to_le_bytes());
        offset += 8;
        buf[offset] = self.match_event_indicator;
        offset += 1;
        buf[offset] = 0; // padding
        offset += 1;

        let count = self.entries.len() as u16;
        buf[offset..offset + 2].copy_from_slice(&count.to_le_bytes());
        offset += 2;

        for entry in &self.entries {
            entry.encode(&mut buf[offset..offset + BookEntry::SIZE])?;
            offset += BookEntry::SIZE;
        }

        Ok(offset)
    }

    /// Decode from buffer.
    pub fn decode(buf: &[u8]) -> Result<Self, ExchangeCodecError> {
        if buf.len() < MessageHeader::SIZE + (Self::ROOT_BLOCK_LEN as usize) {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: MessageHeader::SIZE + (Self::ROOT_BLOCK_LEN as usize),
                actual: buf.len(),
            });
        }
        let header = MessageHeader::decode(&buf[0..MessageHeader::SIZE])?;
        if header.template_id != Self::TEMPLATE_ID {
            return Err(ExchangeCodecError::UnknownMessageType(header.template_id));
        }

        let mut offset = MessageHeader::SIZE;
        let transact_time = u64::from_le_bytes([
            buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3],
            buf[offset + 4], buf[offset + 5], buf[offset + 6], buf[offset + 7],
        ]);
        offset += 8;
        let match_event = buf[offset];
        offset += 2; // match_event + pad

        let num_entries = u16::from_le_bytes([buf[offset], buf[offset + 1]]) as usize;
        offset += 2;

        let required = offset + num_entries * BookEntry::SIZE;
        if buf.len() < required {
            return Err(ExchangeCodecError::BufferUnderflow {
                expected: required,
                actual: buf.len(),
            });
        }

        let mut entries = Vec::with_capacity(num_entries);
        for _ in 0..num_entries {
            let entry = BookEntry::decode(&buf[offset..offset + BookEntry::SIZE])?;
            entries.push(entry);
            offset += BookEntry::SIZE;
        }

        Ok(Self {
            transact_time_nanos: transact_time,
            match_event_indicator: match_event,
            entries,
        })
    }
}
