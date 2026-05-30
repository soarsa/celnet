//! Celnet durable event journal — the standalone crash-recovery substrate
//! (`docs/SCALE-OUT.md` §8; closes the "designed-only" durable-log gap).
//!
//! An **fsync'd, append-only, sequence-ordered** log of the must-order durable
//! events (accepted market-state updates + the quote/trade lifecycle). The engine
//! appends to it; on restart it **replays** the log to rebuild the live book /
//! marked-surface state **bit-identically** (determinism: libm + counter-based
//! RNG ⇒ identical reconstruction). Each record carries a monotonic sequence and
//! a checksum; a torn tail (crash mid-append) is detected and truncated cleanly so
//! recovery never reads a partial record. Skeleton — implementation lands here.
#![forbid(unsafe_code)]
