//! Celnet **risk transfer** — the pure, asset-agnostic domain of moving
//! **existing** risk between books, desks, and traders. It is the exact
//! complement to **risk routing**: routing is the *automatic* assignment of
//! *new* fills to a risk portfolio at book time; a transfer is the *manual* move
//! of *already-open* risk.
//!
//! This leaf crate is **server-, wire-, and market-data-free**: plain owned Rust
//! types, deterministic validation, and a deterministic leg computation. The
//! server layer (a later phase) populates the small [`TransferContext`] input
//! from its position store / identity registries and applies the computed legs
//! through the existing booking sinks; nothing here touches the pinned pricing
//! hot core.
//!
//! # The two operations (locks the taxonomy)
//!
//! Industry practice separates two fundamentally different moves, and Celnet
//! must too (see `docs/RISK-TRANSFER-REQUIREMENTS.md` §3–4):
//!
//! - **Re-attribution** ([`TransferKind::ReAttribute`]) — the position is
//!   re-labelled to a different portfolio **within the same desk**. Economics
//!   are unchanged, no P&L crosses, no new trade. A re-stamp of the routing
//!   dimension.
//! - **Economic internal trade** ([`TransferKind::DeskToDesk`] /
//!   [`TransferKind::TraderToTrader`] across books) — an **internal cross** at a
//!   **transfer price**: two offsetting bookings that **realise P&L in the
//!   source** and **open risk in the target** at the agreed mark. This is a
//!   *book pair*, not a re-label; getting the transfer price right (arm's-length
//!   / mark) protects P&L integrity.
//!
//! # The leg mechanic (mirror / back-to-back)
//!
//! An economic transfer books **two offsetting legs** — an offsetting position
//! out of the source (opposite side, moved notional) matched by an opening
//! position into the target (original side, moved notional). The pair **nets to
//! zero at the firm level** while moving risk and P&L between books. This crate
//! computes that pair deterministically:
//!
//! ```text
//! source offsetting leg : signed notional = −(moved aggregate)   @ transfer price
//! target opening   leg : signed notional = +(moved aggregate)   @ transfer price
//! realized P&L (source) : Σ  moved_i · (transfer_price − mark_i)
//! moved risk           : f · Σ risk_i         (f = moved fraction of each slice)
//! ```
//!
//! Sign convention: a **positive** signed notional is **long**, **negative** is
//! **short**. `Full` moves the whole aggregate (`f = 1`); `Partial(q)` moves a
//! notional magnitude `q` pro-rata across the selected slices
//! (`f = q / |Σ signed_notional|`), preserving each slice's side.
//!
//! # Purity
//!
//! [`compute_legs`] and [`plan_position_moves`] are pure functions of their
//! inputs — no clock, no rng, no I/O — so they are deterministic and
//! oracle-testable against a hand-written truth table (guardrail 5).

mod context;
mod error;
mod legs;
mod provenance;
mod risk;
mod transfer;
mod validate;

pub use context::{BookRef, PositionRef, TransferContext};
pub use error::TransferError;
pub use legs::{
    BookedLeg, LegRole, MoveKind, PositionMove, PositionSlice, TransferLegs, compute_legs,
    plan_position_moves,
};
pub use provenance::{PriceBasis, RiskTransferProvenance};
pub use risk::{MovedRisk, RiskVector};
pub use transfer::{
    RiskTransfer, TransferKind, TransferLeg, TransferPrice, TransferQuantity, TransferState,
};
pub use validate::check_transfer;
