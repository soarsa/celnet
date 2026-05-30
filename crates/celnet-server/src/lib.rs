//! Celnet service edge — the tokio async front (gRPC via tonic + WebSocket RFS
//! streaming) that exposes the core-pinned [`celnet_engine`] pricing core to the
//! Celer estate and front end, joined to the hot path only by wait-free rings
//! (work-stream WS-I). Skeleton — implementation lands in this lane.
#![forbid(unsafe_code)]
