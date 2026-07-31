//! The announce → elect → confirm → apply lifecycle (§7.5) that drives pricing + position off a
//! confirmed corporate action.
//!
//! Applying a confirmed CA event does two things atomically from the caller's view:
//! 1. **Re-derives the instrument's schedule** — [`celnet_corpactions::apply_event`] transforms the
//!    stored [`crate::master::InstrumentMaster`] schedule and a new effective-dated version is
//!    appended, so bond pricing (`celnet-bond` `CashflowSchedule::from_bond`) and DV01 / key-rate
//!    (`celnet-rates-risk` `ladder.rs`) re-derive off the **post-event** schedule automatically.
//! 2. **Books the position effect** — [`celnet_corpactions::position_delta`] is pushed to the
//!    [`PositionSink`] (the server's `RatesPositionStore`), so REDM/MCAL realise and PCAL/PRED/DRAW
//!    scale the held nominal through the same booking path a trade uses (§8), never a parallel store.
//!
//! A reversal (seev.037) walks the append-only history to restore the pre-event schedule and re-books
//! the inverse delta; nothing is deleted (§7.3, the double-count guard §10.2). All transitions are
//! gated by [`CaStatus::can_transition_to`]. Off the pinned pricing hot core (guardrail 11).

use celnet_corpactions::{
    CaStatus, CivilDate, PositionDelta, PositionEffect, apply_event, position_delta,
};

use crate::master::{InstrumentMaster, Provenance, SourceRef};
use crate::sink::{PositionSink, SinkError};
use crate::store::{GoldenSourceStore, StoreError, StoredCorpAction};

/// Failure modes of a lifecycle transition.
#[derive(Debug)]
pub enum LifecycleError {
    /// No corporate action with that id is known.
    UnknownCorpAction(String),
    /// The event's ISIN resolves to no instrument in the store.
    UnknownInstrument(String),
    /// The requested status transition is not legal from the current status.
    IllegalTransition {
        /// The current status.
        from: CaStatus,
        /// The requested status.
        to: CaStatus,
    },
    /// A reversal was requested but no pre-event instrument version exists to restore.
    NothingToReverse(String),
    /// The pure effect computation rejected the event against the schedule.
    Apply(celnet_corpactions::CaError),
    /// The position sink rejected the delta.
    Sink(SinkError),
    /// The durable store failed.
    Store(StoreError),
}

impl core::fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownCorpAction(id) => write!(f, "unknown corporate action {id}"),
            Self::UnknownInstrument(isin) => write!(f, "no instrument for ISIN {isin}"),
            Self::IllegalTransition { from, to } => {
                write!(f, "illegal CA transition {from:?} -> {to:?}")
            }
            Self::NothingToReverse(id) => write!(f, "no pre-event version to reverse for {id}"),
            Self::Apply(e) => write!(f, "corporate-action effect rejected: {e}"),
            Self::Sink(e) => write!(f, "position sink rejected the delta: {e}"),
            Self::Store(e) => write!(f, "golden-source store failed: {e}"),
        }
    }
}

impl core::error::Error for LifecycleError {}

impl From<StoreError> for LifecycleError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}
impl From<SinkError> for LifecycleError {
    fn from(e: SinkError) -> Self {
        Self::Sink(e)
    }
}

/// The result of applying (or reversing) a corporate action.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyOutcome {
    /// The instrument the effect touched.
    pub instrument_id: String,
    /// The position transform shape.
    pub effect: PositionEffect,
    /// The concrete position delta booked to the sink.
    pub delta: PositionDelta,
    /// The number of cashflows remaining in the instrument's post-event schedule.
    pub remaining_flows: usize,
}

/// Record a superseding CA version with a new `status`, after checking the transition is legal.
fn transition(
    store: &mut GoldenSourceStore,
    ca_id: &str,
    to: CaStatus,
    provenance: Provenance,
) -> Result<StoredCorpAction, LifecycleError> {
    let current = store
        .latest_corp_action(ca_id)
        .ok_or_else(|| LifecycleError::UnknownCorpAction(ca_id.to_string()))?;
    if !current.event.status.can_transition_to(to) {
        return Err(LifecycleError::IllegalTransition {
            from: current.event.status,
            to,
        });
    }
    let mut next = current.clone();
    next.event.status = to;
    next.provenance = provenance;
    store.record_corp_action(next.clone())?;
    Ok(next)
}

/// Announce a corporate action — the first append (status `Announced`), typically the source's own
/// normalized record.
///
/// # Errors
/// [`LifecycleError::Store`] on a durable-write failure.
pub fn announce(store: &mut GoldenSourceStore, ca: StoredCorpAction) -> Result<(), LifecycleError> {
    store.record_corp_action(ca)?;
    Ok(())
}

/// Resolve a VOLU/CHOS election (`Announced -> Elected`).
///
/// # Errors
/// [`LifecycleError::IllegalTransition`] if the CA is not `Announced`; store failures.
pub fn elect(
    store: &mut GoldenSourceStore,
    ca_id: &str,
    provenance: Provenance,
) -> Result<(), LifecycleError> {
    transition(store, ca_id, CaStatus::Elected, provenance).map(|_| ())
}

/// Confirm a movement (`Announced|Elected -> Confirmed`).
///
/// # Errors
/// [`LifecycleError::IllegalTransition`] if not confirmable; store failures.
pub fn confirm(
    store: &mut GoldenSourceStore,
    ca_id: &str,
    provenance: Provenance,
) -> Result<(), LifecycleError> {
    transition(store, ca_id, CaStatus::Confirmed, provenance).map(|_| ())
}

/// Cancel an announced/elected/confirmed event before application (`-> Cancelled`).
///
/// # Errors
/// [`LifecycleError::IllegalTransition`] if not cancellable; store failures.
pub fn cancel(
    store: &mut GoldenSourceStore,
    ca_id: &str,
    provenance: Provenance,
) -> Result<(), LifecycleError> {
    transition(store, ca_id, CaStatus::Cancelled, provenance).map(|_| ())
}

/// Apply a **confirmed** corporate action: transform the stored schedule and book the position effect.
///
/// `held_face` is the desk's inventory in the instrument (the CA acts on the proprietary book). A new
/// effective-dated instrument version (post-event schedule) and a superseding `Applied` CA version are
/// appended; the position delta is pushed to `sink`.
///
/// # Errors
/// [`LifecycleError::UnknownCorpAction`] / [`LifecycleError::UnknownInstrument`] /
/// [`LifecycleError::IllegalTransition`] (CA not `Confirmed`) / [`LifecycleError::Apply`] /
/// [`LifecycleError::Sink`] / [`LifecycleError::Store`].
pub fn apply<S: PositionSink>(
    store: &mut GoldenSourceStore,
    sink: &mut S,
    ca_id: &str,
    held_face: f64,
    valid_from: CivilDate,
    recorded_at: CivilDate,
) -> Result<ApplyOutcome, LifecycleError> {
    let ca = store
        .latest_corp_action(ca_id)
        .ok_or_else(|| LifecycleError::UnknownCorpAction(ca_id.to_string()))?
        .clone();
    if ca.event.status != CaStatus::Confirmed {
        return Err(LifecycleError::IllegalTransition {
            from: ca.event.status,
            to: CaStatus::Applied,
        });
    }
    let master = store
        .latest_instrument_by_isin(&ca.event.isin)
        .ok_or_else(|| LifecycleError::UnknownInstrument(ca.event.isin.clone()))?
        .clone();

    let applied = apply_event(&master.schedule, &ca.event).map_err(LifecycleError::Apply)?;
    let delta = position_delta(&applied.effect, held_face);

    // 1. New effective-dated instrument version off the post-event schedule (pricing re-derives here).
    let new_master = InstrumentMaster {
        schedule: applied.schedule.clone(),
        provenance: Provenance {
            source: SourceRef("ca_apply".to_string()),
            valid_from,
            recorded_at,
            ..master.provenance
        },
        ..master.clone()
    };
    store.upsert_instrument(new_master)?;

    // 2. Book the position effect through the sink (same path a trade uses).
    sink.apply_delta(&master.instrument_id, &delta)?;

    // 3. Supersede the CA as Applied.
    transition(
        store,
        ca_id,
        CaStatus::Applied,
        Provenance {
            source: SourceRef("ca_apply".to_string()),
            valid_from,
            recorded_at,
            ..ca.provenance
        },
    )?;

    Ok(ApplyOutcome {
        instrument_id: master.instrument_id,
        effect: applied.effect,
        delta,
        remaining_flows: applied.schedule.len(),
    })
}

/// Reverse the most recent **applied** movement of a corporate action (seev.037): restore the
/// pre-event schedule and re-book the inverse position delta. Nothing is deleted — a superseding
/// `Reversed` version is appended and a fresh instrument version restoring the schedule is written.
///
/// # Errors
/// [`LifecycleError::IllegalTransition`] if the CA is not `Applied`; [`LifecycleError::NothingToReverse`]
/// if no pre-event instrument version exists; effect / sink / store failures.
pub fn reverse<S: PositionSink>(
    store: &mut GoldenSourceStore,
    sink: &mut S,
    ca_id: &str,
    held_face: f64,
    valid_from: CivilDate,
    recorded_at: CivilDate,
) -> Result<ApplyOutcome, LifecycleError> {
    let ca = store
        .latest_corp_action(ca_id)
        .ok_or_else(|| LifecycleError::UnknownCorpAction(ca_id.to_string()))?
        .clone();
    if ca.event.status != CaStatus::Applied {
        return Err(LifecycleError::IllegalTransition {
            from: ca.event.status,
            to: CaStatus::Reversed,
        });
    }
    let master = store
        .latest_instrument_by_isin(&ca.event.isin)
        .ok_or_else(|| LifecycleError::UnknownInstrument(ca.event.isin.clone()))?
        .clone();
    // The pre-event schedule is the version immediately before the apply's version.
    let versions = store.instrument_versions(&master.instrument_id);
    if versions.len() < 2 {
        return Err(LifecycleError::NothingToReverse(ca_id.to_string()));
    }
    let pre = versions[versions.len() - 2].clone();

    // Re-run the effect on the pre-event schedule → the same delta the apply booked; invert it.
    let applied = apply_event(&pre.schedule, &ca.event).map_err(LifecycleError::Apply)?;
    let forward = position_delta(&applied.effect, held_face);
    let inverse = PositionDelta {
        face_delta: -forward.face_delta,
        cash: -forward.cash,
        exchange_into: forward
            .exchange_into
            .as_ref()
            .map(|leg| celnet_corpactions::ExchangeLeg {
                target: leg.target.clone(),
                units: -leg.units,
            }),
    };

    // Restore the pre-event schedule as a new effective version; re-book the inverse delta.
    let restored = InstrumentMaster {
        schedule: pre.schedule.clone(),
        provenance: Provenance {
            source: SourceRef("ca_reverse".to_string()),
            valid_from,
            recorded_at,
            ..pre.provenance
        },
        ..pre.clone()
    };
    store.upsert_instrument(restored)?;
    sink.apply_delta(&master.instrument_id, &inverse)?;

    transition(
        store,
        ca_id,
        CaStatus::Reversed,
        Provenance {
            source: SourceRef("ca_reverse".to_string()),
            valid_from,
            recorded_at,
            ..ca.provenance
        },
    )?;

    Ok(ApplyOutcome {
        instrument_id: master.instrument_id,
        effect: applied.effect,
        delta: inverse,
        remaining_flows: pre.schedule.len(),
    })
}
