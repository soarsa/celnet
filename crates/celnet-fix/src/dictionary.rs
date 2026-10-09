//! Table-driven FX-options FIX 4.4 dialect: `tag → type` plus the
//! required / conditional field sets per message type.
//!
//! The dictionary is the single source of truth for which tags exist, what
//! scalar type each carries, and which fields a given `MsgType` requires. It is
//! consulted by [`crate::messages`] (to validate inbound frames) and documented
//! against the §1.2 tag map in `docs/architecture/CELNET-FIX-INTEGRATION-PLAN.md`. The shape
//! follows the dictionary-driven model of `fefix` (cited; not a dependency).

use crate::framing::FrameCursor;

/// The scalar wire type of a FIX field value, used to validate field contents
/// before they are interpreted by the dialect mapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    /// A non-negative integer (e.g. sequence numbers, quantities-as-int).
    Int,
    /// A decimal price/quantity (`f64`).
    Float,
    /// A free-form string (`Symbol`, `QuoteID`, …).
    String,
    /// A single ASCII character enumeration (`PutOrCall`, `Side`, …).
    Char,
    /// A `YYYYMMDD` local-market date.
    LocalMktDate,
    /// A UTC timestamp `YYYYMMDD-HH:MM:SS(.sss)`.
    UtcTimestamp,
    /// A currency code.
    Currency,
}

/// The FX-options dialect message types this engine speaks (`MsgType`, tag 35).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    /// `0` — Heartbeat.
    Heartbeat,
    /// `1` — TestRequest.
    TestRequest,
    /// `2` — ResendRequest.
    ResendRequest,
    /// `3` — Reject.
    Reject,
    /// `4` — SequenceReset (gap-fill / reset).
    SequenceReset,
    /// `5` — Logout.
    Logout,
    /// `A` — Logon.
    Logon,
    /// `R` — QuoteRequest (RFQ / RFS, single or multi-leg).
    QuoteRequest,
    /// `S` — Quote.
    Quote,
    /// `i` — MassQuote.
    MassQuote,
    /// `Z` — QuoteCancel.
    QuoteCancel,
    /// `D` — NewOrderSingle.
    NewOrderSingle,
    /// `AB` — NewOrderMultileg.
    NewOrderMultileg,
    /// `8` — ExecutionReport.
    ExecutionReport,
    /// `AG` — QuoteRequestReject (the desk declined / could not price an RFQ).
    QuoteRequestReject,
    /// `x` — SecurityListRequest (download the tradable-securities universe).
    SecurityListRequest,
    /// `y` — SecurityList (the venue's tradable-securities universe response).
    SecurityList,
    /// `V` — MarketDataRequest (subscribe / unsubscribe a streaming market-data feed).
    MarketDataRequest,
    /// `W` — MarketDataSnapshotFullRefresh (a full top-of-book snapshot per instrument).
    MarketDataSnapshotFullRefresh,
}

impl MsgType {
    /// The on-wire `MsgType` value bytes.
    #[must_use]
    pub const fn as_bytes(self) -> &'static [u8] {
        match self {
            MsgType::Heartbeat => b"0",
            MsgType::TestRequest => b"1",
            MsgType::ResendRequest => b"2",
            MsgType::Reject => b"3",
            MsgType::SequenceReset => b"4",
            MsgType::Logout => b"5",
            MsgType::Logon => b"A",
            MsgType::QuoteRequest => b"R",
            MsgType::Quote => b"S",
            MsgType::MassQuote => b"i",
            MsgType::QuoteCancel => b"Z",
            MsgType::NewOrderSingle => b"D",
            MsgType::NewOrderMultileg => b"AB",
            MsgType::ExecutionReport => b"8",
            MsgType::QuoteRequestReject => b"AG",
            MsgType::SecurityListRequest => b"x",
            MsgType::SecurityList => b"y",
            MsgType::MarketDataRequest => b"V",
            MsgType::MarketDataSnapshotFullRefresh => b"W",
        }
    }

    /// Resolve a `MsgType` value from its wire bytes.
    #[must_use]
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        Some(match b {
            b"0" => MsgType::Heartbeat,
            b"1" => MsgType::TestRequest,
            b"2" => MsgType::ResendRequest,
            b"3" => MsgType::Reject,
            b"4" => MsgType::SequenceReset,
            b"5" => MsgType::Logout,
            b"A" => MsgType::Logon,
            b"R" => MsgType::QuoteRequest,
            b"S" => MsgType::Quote,
            b"i" => MsgType::MassQuote,
            b"Z" => MsgType::QuoteCancel,
            b"D" => MsgType::NewOrderSingle,
            b"AB" => MsgType::NewOrderMultileg,
            b"8" => MsgType::ExecutionReport,
            b"AG" => MsgType::QuoteRequestReject,
            b"x" => MsgType::SecurityListRequest,
            b"y" => MsgType::SecurityList,
            b"V" => MsgType::MarketDataRequest,
            b"W" => MsgType::MarketDataSnapshotFullRefresh,
            _ => return None,
        })
    }

    /// Whether this is a session-layer administrative message (vs application).
    #[must_use]
    pub const fn is_admin(self) -> bool {
        matches!(
            self,
            MsgType::Heartbeat
                | MsgType::TestRequest
                | MsgType::ResendRequest
                | MsgType::Reject
                | MsgType::SequenceReset
                | MsgType::Logout
                | MsgType::Logon
        )
    }
}

/// A dialect tag entry: the tag number, its scalar type, and a human name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagSpec {
    /// FIX tag number.
    pub tag: u32,
    /// Wire scalar type.
    pub field_type: FieldType,
    /// Human-readable field name (for diagnostics; never on the wire).
    pub name: &'static str,
}

/// The full FX-options dialect tag table — the §1.2 / §1.2-multileg map.
///
/// This is `const`, so it incurs no startup cost; lookups are a linear scan
/// over a small table (≈40 entries), which is faster than a hash for this size
/// and keeps the dictionary allocation-free.
pub const TAGS: &[TagSpec] = &[
    // --- session / header ---
    TagSpec {
        tag: 8,
        field_type: FieldType::String,
        name: "BeginString",
    },
    TagSpec {
        tag: 9,
        field_type: FieldType::Int,
        name: "BodyLength",
    },
    TagSpec {
        tag: 35,
        field_type: FieldType::String,
        name: "MsgType",
    },
    TagSpec {
        tag: 49,
        field_type: FieldType::String,
        name: "SenderCompID",
    },
    TagSpec {
        tag: 56,
        field_type: FieldType::String,
        name: "TargetCompID",
    },
    TagSpec {
        tag: 34,
        field_type: FieldType::Int,
        name: "MsgSeqNum",
    },
    TagSpec {
        tag: 52,
        field_type: FieldType::UtcTimestamp,
        name: "SendingTime",
    },
    TagSpec {
        tag: 43,
        field_type: FieldType::Char,
        name: "PossDupFlag",
    },
    TagSpec {
        tag: 122,
        field_type: FieldType::UtcTimestamp,
        name: "OrigSendingTime",
    },
    TagSpec {
        tag: 10,
        field_type: FieldType::Int,
        name: "CheckSum",
    },
    // --- session admin bodies ---
    TagSpec {
        tag: 98,
        field_type: FieldType::Int,
        name: "EncryptMethod",
    },
    TagSpec {
        tag: 108,
        field_type: FieldType::Int,
        name: "HeartBtInt",
    },
    TagSpec {
        tag: 141,
        field_type: FieldType::Char,
        name: "ResetSeqNumFlag",
    },
    TagSpec {
        tag: 112,
        field_type: FieldType::String,
        name: "TestReqID",
    },
    TagSpec {
        tag: 7,
        field_type: FieldType::Int,
        name: "BeginSeqNo",
    },
    TagSpec {
        tag: 16,
        field_type: FieldType::Int,
        name: "EndSeqNo",
    },
    TagSpec {
        tag: 36,
        field_type: FieldType::Int,
        name: "NewSeqNo",
    },
    TagSpec {
        tag: 123,
        field_type: FieldType::Char,
        name: "GapFillFlag",
    },
    TagSpec {
        tag: 58,
        field_type: FieldType::String,
        name: "Text",
    },
    TagSpec {
        tag: 45,
        field_type: FieldType::Int,
        name: "RefSeqNum",
    },
    TagSpec {
        tag: 372,
        field_type: FieldType::String,
        name: "RefMsgType",
    },
    // --- instrument block (§1.2) ---
    TagSpec {
        tag: 55,
        field_type: FieldType::String,
        name: "Symbol",
    },
    TagSpec {
        tag: 460,
        field_type: FieldType::Int,
        name: "Product",
    },
    TagSpec {
        tag: 461,
        field_type: FieldType::String,
        name: "CFICode",
    },
    TagSpec {
        tag: 167,
        field_type: FieldType::String,
        name: "SecurityType",
    },
    TagSpec {
        tag: 201,
        field_type: FieldType::Int,
        name: "PutOrCall",
    },
    TagSpec {
        tag: 202,
        field_type: FieldType::Float,
        name: "StrikePrice",
    },
    TagSpec {
        tag: 947,
        field_type: FieldType::Currency,
        name: "StrikeCurrency",
    },
    TagSpec {
        tag: 1194,
        field_type: FieldType::Int,
        name: "ExerciseStyle",
    },
    TagSpec {
        tag: 541,
        field_type: FieldType::LocalMktDate,
        name: "MaturityDate",
    },
    TagSpec {
        tag: 1079,
        field_type: FieldType::String,
        name: "MaturityTime",
    },
    TagSpec {
        tag: 64,
        field_type: FieldType::LocalMktDate,
        name: "SettlDate",
    },
    TagSpec {
        tag: 15,
        field_type: FieldType::Currency,
        name: "Currency",
    },
    TagSpec {
        tag: 120,
        field_type: FieldType::Currency,
        name: "SettlCurrency",
    },
    TagSpec {
        tag: 38,
        field_type: FieldType::Float,
        name: "OrderQty",
    },
    TagSpec {
        tag: 152,
        field_type: FieldType::Float,
        name: "CashOrderQty",
    },
    // --- quote / pricing ---
    TagSpec {
        tag: 131,
        field_type: FieldType::String,
        name: "QuoteReqID",
    },
    TagSpec {
        tag: 117,
        field_type: FieldType::String,
        name: "QuoteID",
    },
    TagSpec {
        tag: 693,
        field_type: FieldType::String,
        name: "QuoteRespID",
    },
    TagSpec {
        tag: 537,
        field_type: FieldType::Int,
        name: "QuoteType",
    },
    TagSpec {
        tag: 132,
        field_type: FieldType::Float,
        name: "BidPx",
    },
    TagSpec {
        tag: 133,
        field_type: FieldType::Float,
        name: "OfferPx",
    },
    TagSpec {
        tag: 134,
        field_type: FieldType::Float,
        name: "BidSize",
    },
    TagSpec {
        tag: 135,
        field_type: FieldType::Float,
        name: "OfferSize",
    },
    TagSpec {
        tag: 423,
        field_type: FieldType::Int,
        name: "PriceType",
    },
    TagSpec {
        tag: 62,
        field_type: FieldType::UtcTimestamp,
        name: "ValidUntilTime",
    },
    // --- order / exec ---
    TagSpec {
        tag: 11,
        field_type: FieldType::String,
        name: "ClOrdID",
    },
    TagSpec {
        tag: 54,
        field_type: FieldType::Char,
        name: "Side",
    },
    TagSpec {
        tag: 60,
        field_type: FieldType::UtcTimestamp,
        name: "TransactTime",
    },
    TagSpec {
        tag: 40,
        field_type: FieldType::Char,
        name: "OrdType",
    },
    // The order's time-in-force. Declared here so an inbound value is type-checked
    // by `validate` rather than being read as an opaque byte: a venue that cannot
    // honour a resting TIF must be able to tell a MALFORMED order from a
    // well-formed one it declines, and answer each differently.
    TagSpec {
        tag: 59,
        field_type: FieldType::Char,
        name: "TimeInForce",
    },
    TagSpec {
        tag: 44,
        field_type: FieldType::Float,
        name: "Price",
    },
    TagSpec {
        tag: 37,
        field_type: FieldType::String,
        name: "OrderID",
    },
    TagSpec {
        tag: 17,
        field_type: FieldType::String,
        name: "ExecID",
    },
    TagSpec {
        tag: 150,
        field_type: FieldType::Char,
        name: "ExecType",
    },
    TagSpec {
        tag: 39,
        field_type: FieldType::Char,
        name: "OrdStatus",
    },
    TagSpec {
        tag: 32,
        field_type: FieldType::Float,
        name: "LastQty",
    },
    TagSpec {
        tag: 31,
        field_type: FieldType::Float,
        name: "LastPx",
    },
    TagSpec {
        tag: 442,
        field_type: FieldType::Int,
        name: "MultiLegReportingType",
    },
    // --- multileg group (QuotReqLegsGrp / legs) ---
    TagSpec {
        tag: 555,
        field_type: FieldType::Int,
        name: "NoLegs",
    },
    TagSpec {
        tag: 600,
        field_type: FieldType::String,
        name: "LegSymbol",
    },
    TagSpec {
        tag: 608,
        field_type: FieldType::String,
        name: "LegCFICode",
    },
    TagSpec {
        tag: 609,
        field_type: FieldType::String,
        name: "LegSecurityType",
    },
    TagSpec {
        tag: 612,
        field_type: FieldType::Float,
        name: "LegStrikePrice",
    },
    TagSpec {
        tag: 624,
        field_type: FieldType::Char,
        name: "LegSide",
    },
    TagSpec {
        tag: 623,
        field_type: FieldType::Float,
        name: "LegRatioQty",
    },
    TagSpec {
        tag: 556,
        field_type: FieldType::Currency,
        name: "LegCurrency",
    },
    TagSpec {
        tag: 654,
        field_type: FieldType::String,
        name: "LegRefID",
    },
    TagSpec {
        tag: 611,
        field_type: FieldType::LocalMktDate,
        name: "LegMaturityDate",
    },
    TagSpec {
        tag: 1358,
        field_type: FieldType::Int,
        name: "LegPutOrCall",
    },
    // --- security-list request / response (35=x / 35=y) ---
    TagSpec {
        tag: 320,
        field_type: FieldType::String,
        name: "SecurityReqID",
    },
    TagSpec {
        tag: 559,
        field_type: FieldType::Int,
        name: "SecurityListRequestType",
    },
    TagSpec {
        tag: 560,
        field_type: FieldType::Int,
        name: "SecurityRequestResult",
    },
    TagSpec {
        tag: 393,
        field_type: FieldType::Int,
        name: "TotNoRelatedSym",
    },
    TagSpec {
        tag: 146,
        field_type: FieldType::Int,
        name: "NoRelatedSym",
    },
    TagSpec {
        tag: 893,
        field_type: FieldType::Char,
        name: "LastFragment",
    },
    // --- market-data (35=V MarketDataRequest / 35=W MarketDataSnapshotFullRefresh) ---
    TagSpec {
        tag: 262,
        field_type: FieldType::String,
        name: "MDReqID",
    },
    TagSpec {
        tag: 263,
        field_type: FieldType::Char,
        name: "SubscriptionRequestType",
    },
    TagSpec {
        tag: 264,
        field_type: FieldType::Int,
        name: "MarketDepth",
    },
    TagSpec {
        tag: 265,
        field_type: FieldType::Int,
        name: "MDUpdateType",
    },
    TagSpec {
        tag: 267,
        field_type: FieldType::Int,
        name: "NoMDEntryTypes",
    },
    TagSpec {
        tag: 269,
        field_type: FieldType::Char,
        name: "MDEntryType",
    },
    TagSpec {
        tag: 268,
        field_type: FieldType::Int,
        name: "NoMDEntries",
    },
    TagSpec {
        tag: 270,
        field_type: FieldType::Float,
        name: "MDEntryPx",
    },
    TagSpec {
        tag: 271,
        field_type: FieldType::Float,
        name: "MDEntrySize",
    },
];

/// Look up the [`TagSpec`] for a tag number, if it is in the dialect.
#[must_use]
pub fn tag_spec(tag: u32) -> Option<&'static TagSpec> {
    TAGS.iter().find(|t| t.tag == tag)
}

/// The required body tags for a given message type. These are the minimal
/// fields the dialect mapper depends on. Conditionally-required fields (e.g.
/// `OrigSendingTime(122)`, mandatory only when `PossDupFlag(43)=Y`) are not
/// listed here; they are enforced contextually in [`validate`].
#[must_use]
pub const fn required_tags(mt: MsgType) -> &'static [u32] {
    match mt {
        MsgType::Heartbeat => &[35],
        MsgType::TestRequest => &[35, 112],
        MsgType::ResendRequest => &[35, 7, 16],
        MsgType::Reject => &[35, 45],
        MsgType::SequenceReset => &[35, 36],
        MsgType::Logout => &[35],
        MsgType::Logon => &[35, 98, 108],
        // Single-leg RFQ needs the instrument; multileg uses NoLegs(555).
        MsgType::QuoteRequest => &[35, 131],
        MsgType::Quote => &[35, 117],
        MsgType::MassQuote => &[35, 117],
        MsgType::QuoteCancel => &[35, 117],
        MsgType::NewOrderSingle => &[35, 11, 54, 38],
        MsgType::NewOrderMultileg => &[35, 11, 555],
        MsgType::ExecutionReport => &[35, 37, 17, 150, 39],
        // A reject must address the originating request (QuoteReqID).
        MsgType::QuoteRequestReject => &[35, 131],
        // Both sides of the security-list exchange carry SecurityReqID(320); the
        // request states its type (559), the response its result (560).
        MsgType::SecurityListRequest => &[35, 320, 559],
        MsgType::SecurityList => &[35, 320, 560],
        // A MarketDataRequest carries the request id (262), the subscribe/unsubscribe
        // intent (263), and at least one instrument in the NoRelatedSym(146) group.
        MsgType::MarketDataRequest => &[35, 262, 263, 146],
        // A full-refresh snapshot echoes the request id (262), names the instrument (55),
        // and carries the market-data entries group (268).
        MsgType::MarketDataSnapshotFullRefresh => &[35, 262, 55, 268],
    }
}

/// Verify that every required tag for the frame's `MsgType` is present, and
/// that each present dialect field's value parses as its declared type. Returns
/// the resolved [`MsgType`].
///
/// Also enforces the conditional `OrigSendingTime(122)` rule: a possible
/// duplicate (`PossDupFlag(43)=Y`) — i.e. a resent message — MUST carry
/// `OrigSendingTime(122)` per FIX, since the receiver needs the original send
/// time to disambiguate the replay. A `PossDupFlag=Y` frame without it is
/// rejected.
///
/// # Errors
/// Returns a [`DictError`] when the message type is unknown, a required field
/// is missing, a `PossDupFlag=Y` resend lacks `OrigSendingTime`, or a field
/// value fails its type check.
pub fn validate(frame: &FrameCursor<'_>) -> Result<MsgType, DictError> {
    let mt = MsgType::from_bytes(frame.msg_type()).ok_or(DictError::UnknownMsgType)?;

    for &req in required_tags(mt) {
        if req == 35 {
            continue; // MsgType presence is guaranteed by the framer.
        }
        if frame.get(req).is_none() {
            return Err(DictError::MissingRequired { tag: req });
        }
    }

    // Conditional requirement: a possible-duplicate resend (PossDupFlag=Y) must
    // carry OrigSendingTime(122).
    if frame.get(43) == Some(b"Y") && frame.get(122).is_none() {
        return Err(DictError::MissingRequired { tag: 122 });
    }

    for field in frame.fields() {
        if let Some(spec) = tag_spec(field.tag)
            && !type_ok(spec.field_type, field.value)
        {
            return Err(DictError::BadType {
                tag: field.tag,
                field_type: spec.field_type,
            });
        }
        // Unknown tags are tolerated (custom/user-defined) — the dialect maps
        // only the tags it understands, per the single-contract guardrail.
    }
    Ok(mt)
}

/// Whether a value's bytes are well-formed for a given scalar type.
fn type_ok(ft: FieldType, v: &[u8]) -> bool {
    if v.is_empty() {
        return false;
    }
    match ft {
        FieldType::Int => v.iter().all(u8::is_ascii_digit),
        FieldType::Float => float_ok(v),
        FieldType::Char => v.len() == 1,
        FieldType::Currency => v.len() == 3 && v.iter().all(u8::is_ascii_alphabetic),
        FieldType::LocalMktDate => v.len() == 8 && v.iter().all(u8::is_ascii_digit),
        // String and timestamp are accepted as any non-empty printable run; the
        // dialect mapper re-parses timestamps where it needs them.
        FieldType::String | FieldType::UtcTimestamp => true,
    }
}

/// Strict FIX float: optional leading `-`, digits, optional single `.`, digits.
fn float_ok(v: &[u8]) -> bool {
    let mut bytes = v;
    if bytes.first() == Some(&b'-') {
        bytes = &bytes[1..];
    }
    if bytes.is_empty() {
        return false;
    }
    let mut seen_dot = false;
    let mut seen_digit = false;
    for &b in bytes {
        match b {
            b'0'..=b'9' => seen_digit = true,
            b'.' if !seen_dot => seen_dot = true,
            _ => return false,
        }
    }
    seen_digit
}

/// A dictionary validation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictError {
    /// The `MsgType` is not part of the FX-options dialect.
    UnknownMsgType,
    /// A required field for the message type was absent.
    MissingRequired {
        /// The missing tag number.
        tag: u32,
    },
    /// A present field's value did not parse as its declared scalar type.
    BadType {
        /// The offending tag.
        tag: u32,
        /// The type it was expected to satisfy.
        field_type: FieldType,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::FrameEncoder;

    #[test]
    fn msgtype_roundtrips() {
        for mt in [
            MsgType::Heartbeat,
            MsgType::Logon,
            MsgType::QuoteRequest,
            MsgType::Quote,
            MsgType::MassQuote,
            MsgType::NewOrderSingle,
            MsgType::NewOrderMultileg,
            MsgType::ExecutionReport,
            MsgType::MarketDataRequest,
            MsgType::MarketDataSnapshotFullRefresh,
        ] {
            assert_eq!(MsgType::from_bytes(mt.as_bytes()), Some(mt));
        }
        assert_eq!(MsgType::from_bytes(b"V"), Some(MsgType::MarketDataRequest));
        assert_eq!(
            MsgType::from_bytes(b"W"),
            Some(MsgType::MarketDataSnapshotFullRefresh)
        );
        assert_eq!(MsgType::from_bytes(b"ZZZ"), None);
    }

    #[test]
    fn tag_table_has_no_duplicates() {
        for (i, a) in TAGS.iter().enumerate() {
            for b in &TAGS[i + 1..] {
                assert_ne!(a.tag, b.tag, "duplicate tag {} in dialect table", a.tag);
            }
        }
    }

    #[test]
    fn float_validation() {
        assert!(float_ok(b"1.0950"));
        assert!(float_ok(b"-0.5"));
        assert!(float_ok(b"100"));
        assert!(!float_ok(b""));
        assert!(!float_ok(b"1.2.3"));
        assert!(!float_ok(b"abc"));
        assert!(!float_ok(b"-"));
    }

    #[test]
    fn validate_rejects_missing_required() {
        let mut e = FrameEncoder::new();
        e.push(35, b"D"); // NewOrderSingle, but missing ClOrdID/Side/OrderQty
        let raw = e.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        let err = validate(&frame).unwrap_err();
        assert!(matches!(err, DictError::MissingRequired { .. }));
    }

    #[test]
    fn validate_rejects_bad_type() {
        let mut e = FrameEncoder::new();
        e.push(35, b"R");
        e.push(131, b"REQ1");
        e.push(202, b"not-a-number"); // StrikePrice must be Float
        let raw = e.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        let err = validate(&frame).unwrap_err();
        assert!(matches!(err, DictError::BadType { tag: 202, .. }));
    }

    #[test]
    fn validate_accepts_well_formed() {
        let mut e = FrameEncoder::new();
        e.push(35, b"R");
        e.push(131, b"REQ1");
        e.push(55, b"EURUSD");
        e.push(201, b"1");
        e.push(202, b"1.0950");
        let raw = e.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        assert_eq!(validate(&frame).unwrap(), MsgType::QuoteRequest);
    }

    #[test]
    fn validate_rejects_possdup_without_orig_sending_time() {
        // A heartbeat marked PossDupFlag=Y but missing OrigSendingTime(122)
        // is an invalid resend and must be rejected.
        let mut e = FrameEncoder::new();
        e.push(35, b"0");
        e.push(43, b"Y");
        let raw = e.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        let err = validate(&frame).unwrap_err();
        assert_eq!(err, DictError::MissingRequired { tag: 122 });
    }

    #[test]
    fn validate_accepts_possdup_with_orig_sending_time() {
        let mut e = FrameEncoder::new();
        e.push(35, b"0");
        e.push(43, b"Y");
        e.push(122, b"20260530-12:00:00.000");
        let raw = e.finish();
        let frame = FrameCursor::parse(&raw).unwrap();
        assert_eq!(validate(&frame).unwrap(), MsgType::Heartbeat);
    }
}
