//! Celnet pricing engine — the stateful, non-async, core-pinned hot path:
//! wait-free SPSC ring buffers between the async edge and the busy-poll pricing
//! core, lock-free read-mostly publication (`arc-swap`/seqlock) of the live
//! convention/surface state, zero-allocation pricing/risk, and blue-green
//! upgrade state handoff (work-stream WS-F). Skeleton — implementation lands in
//! this lane.
#![forbid(unsafe_code)]
