//! Celnet fleet router — the horizontal scale-out tier (work-stream WS-J; design
//! in `docs/SCALE-OUT.md`). Routes pricing work across stateless replicas by a
//! deterministic partition map (shard by currency-pair / tenant / book via
//! highest-random-weight hashing), with hot-standby failover and bounded
//! backpressure, so Celnet scales to investment-banking-sized portfolios without
//! routing *through* the pinned hot path. Skeleton — implementation lands here.
#![forbid(unsafe_code)]
