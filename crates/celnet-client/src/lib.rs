//! Celnet typed async client SDK — the ergonomic GUI/API-user surface over the
//! gRPC + WebSocket edge: RFQ (request→quote→accept→execution), RFS streaming
//! (subscribe→snapshot+sequenced deltas+resync), surface reads/marks, and
//! risk/scenario, with reconnect and resync built in (work-stream WS-I/client).
//! The API is evolved by exercising real-like trader workflows as tests. Skeleton
//! — implementation lands in this lane.
#![forbid(unsafe_code)]
