//! `celnet-refstore` — the mastered reference-data + corporate-actions **ingestion & golden-source**
//! layer (Phase B of `docs/fixed-income/BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md`).
//!
//! This is the mutable evolution of the static `celnet-refdata` `Vec` into an **effective-dated,
//! append-only, journal-backed golden source** plus the ingestion machinery that keeps it current
//! and feeds pricing/hedging. It is off the pinned zero-alloc pricing hot core (guardrail 11) and
//! OSS-only (guardrail 7): the govvie/rates path is fully deterministic and derivable in-house, and
//! comprehensive corporate content enters only through a customer-wired vendor adapter — this crate
//! ships **no bundled commercial data**.
//!
//! # The pieces
//!
//! * [`InstrumentMaster`] — the golden record per security (identity + terms + current schedule +
//!   per-field mastering provenance).
//! * [`GoldenSourceStore`] — the effective-dated, append-only, `celnet-journal`-backed store;
//!   bitemporal reads, lineage, and reversal-by-supersession (never delete).
//! * [`RefDataSource`] / [`CorpActionSource`] — the ingestion adapter traits; [`GovvieSource`] is the
//!   deterministic OSS implementation that *derives* schedule-driven CA events from open issuance
//!   terms (`celnet-refdata`), no external feed.
//! * [`PositionSink`] — the seam the server's `RatesPositionStore` implements; [`InMemoryPositionBook`]
//!   is the reference semantics + test double.
//! * [`lifecycle`] — the announce → elect → confirm → apply state machine: applying a confirmed CA
//!   re-derives the stored schedule (so bond pricing + DV01 / key-rate re-derive off the post-event
//!   schedule) and books the position effect through the sink; a reversal restores the pre-event
//!   schedule and re-books the inverse (§8, §10.2 double-count guard).

mod master;
mod sink;
mod source;
mod store;

pub mod lifecycle;

pub use master::{
    CouponType, ExternalIds, InstrumentMaster, InstrumentTerms, Provenance, SourceRef,
};
pub use sink::{Holding, InMemoryPositionBook, PositionSink, SinkError};
pub use source::{CorpActionSource, GovvieSource, RefDataSource, SourceError};
pub use store::{GoldenSourceStore, StoreError, StoredCorpAction};
