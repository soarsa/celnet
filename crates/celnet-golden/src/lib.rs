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
//! * `data/touch_gk.csv` — one-touch / no-touch / double-no-touch / double-touch
//!   references, all priced by QuantLib's `AnalyticDoubleBarrierBinaryEngine`
//!   (single rows via the wide-corridor limit, double rows directly). This is the
//!   *independent* oracle the exotics crate's own complementarity checks cannot
//!   be: `no_touch = df − one_touch` and `double_touch = df − dnt` hold **by
//!   construction** in the Celnet code, so they cannot catch a common-mode error
//!   in the survival series — QuantLib's separate reflection series can. Priced
//!   with [`celnet_exotics::one_touch_price`] /
//!   [`celnet_exotics::no_touch_price`] /
//!   [`celnet_exotics::double_no_touch_price`] /
//!   [`celnet_exotics::double_touch_price`].
//! * `data/double_barrier_gk.csv` — corridor double-barrier knock-out (and the
//!   knock-in complement) of a vanilla payoff, with the knock-out priced by
//!   QuantLib's `AnalyticDoubleBarrierEngine` — an oracle independent of Celnet's
//!   Ikeda-Kunitomo image series — and asserted against
//!   [`celnet_exotics::double_knock_out_price`].
//! * `data/heston_fo.csv` — Heston (1993) stochastic-volatility European-vanilla
//!   reference prices, **hand-pinned from the published literature** (Fang &
//!   Oosterlee 2008, §5.3, the "Reference val." figures of Tables 4 and 5, which
//!   the authors produce by the Carr-Madan method at `N = 2^17`) rather than from
//!   QuantLib — QuantLib is **not** available in this build environment, so this
//!   table is narrowed honestly to authoritative published constants instead of
//!   fabricating an oracle. It is the *independent* oracle the `celnet-heston`
//!   crate's own Carr-Madan-vs-COS cross-check cannot be (two transforms of the
//!   same mis-derived CF could agree on a wrong value; a third-party published
//!   price cannot). Asserted against `celnet_heston::carr_madan` (every row) and
//!   `celnet_heston::cos` (rows within its documented `≤3y` validity).
//!
//! Each table additionally carries a structural self-check (well-formed, finite,
//! sane bounds) so it cannot rot independently of the pricing assertions.
//!
//! [QuantLib]: https://www.quantlib.org/

#![forbid(unsafe_code)]

pub mod csv;
pub mod oracle;
pub mod table;
pub mod vectors;

pub use csv::{CsvError, CsvTable};
pub use table::{
    BarrierRecord, BarrierType, DigitalRecord, DigitalSettlement, DoubleBarrierKind,
    DoubleBarrierRecord, HestonRecord, TouchKind, TouchRecord, VanillaRecord, load_barrier,
    load_digital, load_double_barrier, load_heston, load_touch, load_vanilla,
};
pub use vectors::{
    CROSS_ASSET_FAMILIES, Expected, FAMILIES, GoldenVector, MC_FAMILIES, Market, Tolerance,
    VECTORS_DIR, VectorError, load_cross_asset_vectors, load_vectors, vectors_file,
};

/// Directory holding the frozen reference tables, relative to the crate root.
pub const DATA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data");
