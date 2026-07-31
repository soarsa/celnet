//! `celnet-corpactions` — the vendor-neutral corporate-action **model + pure effect functions**.
//!
//! This leaf crate is the CAEV/CAMV event vocabulary (the open ISO 15022 MT 56x / ISO 20022 `seev.*`
//! code sets Celnet adopts as its internal contract — guardrail 8) and the **deterministic effect
//! math** that transforms a bond's absolute-dated cashflow schedule and a holder's position for each
//! event. It carries **no IO, no clock, no rng, and no server dependency** (guardrail 11: off the
//! pinned hot core), so it compiles and is validated in isolation against an independent hand truth
//! table (`tests/oracle.rs`, guardrail 5).
//!
//! # What is deterministic-real vs adapter-fed (the honest OSS boundary — §2–§3)
//!
//! The *math* for every event is implemented here. What differs is where the *data* comes from:
//!
//! * **Govvie-deterministic** ([`Caev::is_govvie_derivable`] `== true`: REDM/INTR/MCAL/PCAL/PRED/
//!   DRAW/BPUT) — derivable in-house from open issuance terms (US Treasury FiscalData, UK DMO). The
//!   server's deterministic govvie source *generates* these; no external feed is needed.
//! * **Corporate** (TEND/EXOF/CONV) — the effect *shape* is modelled exactly, but the *data*
//!   (announcement, price, ratio, target) has no free comprehensive source and arrives only through
//!   the pluggable vendor-CA adapter a customer wires to their licensed feed.
//!
//! # The effect model
//!
//! A schedule ([`BondSchedule`]) is a list of absolute-dated per-100-face cashflows plus a pool
//! factor. [`apply_event`] maps `(schedule, event) → (new schedule, [`PositionEffect`])`; a coupon
//! settles as income and leaves the future stream intact, a redemption/call empties the schedule,
//! a partial event scales every remaining flow (and the pool factor) by the retained fraction, an
//! exchange empties the source and mints a target leg. [`position_delta`] turns the per-100 effect
//! into the concrete face/cash change on a specific holding — the movement the booking sinks apply
//! (§8), attributed as a corporate action, not a trade (the §10.2 double-count guard).
//!
//! See `docs/BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md` and
//! `docs/ANALYTICS-REQUIREMENTS.md` §10 for the full requirement.

mod date;
mod effect;
mod event;
mod schedule;

pub use date::CivilDate;
pub use effect::{
    AppliedEvent, ExchangeLeg, PositionDelta, PositionEffect, apply_event, position_delta,
};
pub use event::{CaDates, CaEvent, CaStatus, CaTerms, Caev, Camv};
pub use schedule::{BondSchedule, CaError, ScheduleFlow};
