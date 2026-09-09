//! Celnet FIX engine — the real FX-options FIX edge (work-stream WS-I /
//! integration; design in `docs/CELER-FIX-INTEGRATION-PLAN.md`).
//!
//! A complete, hand-rolled, zero-copy FIX 4.4 implementation (no stub, no
//! off-the-shelf engine): SOH framing + BodyLength/CheckSum validation, a
//! FIXT/4.4 session FSM (logon/heartbeat/test-request/resend/gap-fill), the
//! FX-options **dialect** (FXVO/FXNO, PutOrCall/Strike/ExerciseStyle/cut,
//! premium-currency + unit, multi-leg strategy groups) mapped to the
//! `celnet-types` option/strategy vocabulary and convention-checked via
//! `celnet-conventions`, and **both** session roles — an **acceptor** (quote
//! venue: QuoteRequest→Quote/MassQuote, NewOrderSingle/Multileg→ExecutionReport,
//! last-look via QuoteID validity) and an **initiator** (price-taker/hedge).
//!
//! This is a leaf crate consumed only by the async edge (`celnet-server`); the
//! pinned hot path never touches a socket or parser. Tested against REAL peers
//! (acceptor vs initiator over a loopback socket); the dialect is cross-checked
//! to reprice the same premium as the engine's QuantLib-gated pricer. All of the
//! above is implemented and exercised by the crate's unit and integration tests —
//! framing, the FIXT/4.4 session FSM, the FX-options dialect mapping, and both
//! acceptor/initiator roles.
#![forbid(unsafe_code)]

pub mod acceptor;
pub mod backend;
pub mod dialect_cross_asset;
pub mod dialect_fx;
pub mod dialect_rates;
pub mod dictionary;
pub mod framing;
pub mod gateway;
pub mod initiator;
pub mod messages;
pub mod session;
pub mod sim;
pub mod transport;

pub use backend::{
    BackendError, DeskBackend, DeskFill, DeskQuoteOut, DeskRfq, DeskSide, GrpcDeskBackend,
    RequestKind, ResponseOutcome, usd_sofr_curve,
};
pub use dialect_cross_asset::{
    CrossAssetDialectError, CrossAssetOptionRfq, CrossAssetProductKind, CrossAssetQuoteOut,
    PRODUCT_COMMODITY, PRODUCT_DIGITAL_ASSET, PRODUCT_EQUITY, decode_cross_asset_rfq,
    encode_cross_asset_quote,
};
pub use dictionary::MsgType;
pub use framing::{FrameCursor, FrameEncoder, FrameError};
pub use gateway::{DeskGateway, GatewayConfig, GatewayError};
pub use session::{Role, Session, SessionConfig, SessionState};
pub use transport::MAX_FIX_MESSAGE_BYTES;
