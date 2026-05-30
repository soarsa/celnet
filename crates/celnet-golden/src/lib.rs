//! Celnet golden reference validation — frozen QuantLib (open-source oracle)
//! reference tables.
//!
//! This crate is the **numerical gate**: it carries frozen CSV reference tables
//! of FX-option prices and Greeks produced by [QuantLib] (an open-source,
//! BSD-style-licensed analytics library used purely as an offline oracle) and a
//! Rust test suite that asserts the Celnet pricers reproduce those numbers to
//! documented tolerances. The committed CSV is the oracle; the test is the gate.
//!
//! The tables are emitted out-of-band by `tools/goldgen/generate.py` (run under
//! the pinned QuantLib venv) and committed under `data/`. They are **frozen**:
//! they are regenerated only deliberately, never silently, so a drift in any
//! Celnet pricer surfaces as a failing test rather than a quietly-mutated
//! oracle. See `README.md` for the pinned QuantLib version and the regeneration
//! procedure.
//!
//! ## What is checked
//!
//! * `data/vanilla_gk.csv` — Garman-Kohlhagen analytic price plus the Greeks
//!   QuantLib exposes for this engine (spot delta, gamma, vega, theta, both
//!   rhos), across a spot × moneyness × vol × maturity × rate-pair × call/put
//!   grid. The Rust side prices the same inputs with [`celnet_vanilla`] and
//!   asserts closeness via [`celnet_core::is_close`].
//! * `data/barrier_gk.csv` — analytic single-barrier references across all eight
//!   flavours (down/up × in/out × call/put). The Rust side prices each row with
//!   [`celnet_exotics::single_barrier_price`] and asserts closeness. This is the
//!   *independent* oracle the exotics crate's own in/out-parity test cannot be:
//!   parity (`KI + KO = vanilla`) holds by construction in the Celnet code, so it
//!   cannot catch a wrong knock-in block selection — QuantLib can.
//! * `data/digital_gk.csv` — analytic cash-or-nothing and asset-or-nothing
//!   references across both settlement styles × call/put, priced with
//!   [`celnet_exotics::digital_price`] and asserted against QuantLib.
//!
//! Each table additionally carries a structural self-check (well-formed, finite,
//! sane bounds) so it cannot rot independently of the pricing assertions.
//!
//! [QuantLib]: https://www.quantlib.org/

#![forbid(unsafe_code)]

pub mod csv;
pub mod table;

pub use csv::{CsvError, CsvTable};
pub use table::{
    BarrierRecord, BarrierType, DigitalRecord, DigitalSettlement, VanillaRecord, load_barrier,
    load_digital, load_vanilla,
};

/// Directory holding the frozen reference tables, relative to the crate root.
pub const DATA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data");
