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
//! to reprice the same premium as the engine's QuantLib-gated pricer. Skeleton —
//! implementation lands in this lane.
#![forbid(unsafe_code)]
