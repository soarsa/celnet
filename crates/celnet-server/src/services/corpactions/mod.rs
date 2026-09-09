//! The **bond corporate-actions** service edge — the wire front of the
//! `celnet-corpactions` / `celnet-refstore` layer.
//!
//! # What it is
//!
//! `CorporateActionsService` exposes the effective (post-any-applied-CA) instrument
//! schedule, the corporate-action inbox, and the confirm / apply lifecycle drivers over
//! the one current contract. It holds:
//!
//! * a [`GoldenSourceStore`] — the effective-dated, append-only, journal-backed golden
//!   source (bitemporal + lineage; reversals supersede, never delete), seeded from the
//!   deterministic OSS [`GovvieSource`] (masters + the schedule-driven MAND coupon/redemption
//!   events) on first open;
//! * the shared [`RatesPositionStore`] — a confirmed+applied CA books its realising /
//!   scaling movement here through the **same** [`RatesPositionStore::book`] path a trade
//!   uses (no parallel store), so the rates Book workspace reflects the redemption.
//!
//! # The apply seam (pricing + DV01 re-derive off the post-event schedule)
//!
//! `ApplyCorporateAction` drives [`celnet_refstore::lifecycle::apply`]: the stored
//! [`InstrumentMaster`] schedule is transformed by [`celnet_corpactions::apply_event`] and a
//! new effective-dated version is appended, so a bond priced through the reference-data
//! → [`gov_bond_to_instrument_def`](crate::config::reference_data) seam re-derives clean/dirty
//! price, accrued, YTM, DV01, duration and convexity off the **post-event** schedule
//! automatically (`ListInstrumentSchedule` returns exactly that schedule). The concrete
//! position effect ([`celnet_corpactions::position_delta`]) is booked as a realising bond
//! leg into the rates book.
//!
//! # Position-linkage honesty
//!
//! A [`RatesPosition`] carries no ISIN back-reference today, so the CA acts on a caller-supplied
//! `held_face` (mirroring [`celnet_refstore::lifecycle::apply`]'s `held_face`) and books a
//! realising leg sized by the resulting face movement — it does not "find and scale" an existing
//! rates position by instrument. The movement is nonetheless booked through the real
//! [`RatesPositionStore::book`] sink and verified against the [`celnet_corpactions`] oracle.
//!
//! # Auth (deny-by-default)
//!
//! The reads (`ListInstrumentSchedule` / `ListCorporateActions`) require a valid session (the
//! `view` floor). The lifecycle writes (`ConfirmCorporateAction` / `ApplyCorporateAction`) require
//! the dedicated [`Action::Refdata`] capability × [`AssetClass::FixedIncome`] — a narrow
//! reference-data admin authority held back from the default trader bundle
//! (`docs/fixed-income/BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md` §11). A CA apply additionally
//! *books*, still gated on `Book` at the rates sink.

// `tonic::Status` is the contract's typed error; its size is the wire library's choice
// (the same allowance every service module carries).
#![allow(clippy::result_large_err)]

use std::sync::{Arc, Mutex};

use celnet_corpactions::{
    CaStatus, Caev, Camv, CivilDate, PositionDelta, apply_event, position_delta,
};
use celnet_entitlements::{Action, AssetClass, Capability};
use celnet_proto::corporate_actions_service_server::CorporateActionsService;
use celnet_proto::{
    ApplyCorporateActionRequest, ApplyCorporateActionResponse, BondInstrument, BrokenDate,
    ConfirmCorporateActionRequest, ConfirmCorporateActionResponse, CorpActionStatus, CorpEventType,
    CorpMandatory, CorporateActionDesc, InstrumentScheduleFlow, ListCorporateActionsRequest,
    ListCorporateActionsResponse, ListInstrumentScheduleRequest, ListInstrumentScheduleResponse,
    PaymentFrequency, RatesInstrument, RatesPosition, Side, rates_instrument,
};
use celnet_refstore::{
    CorpActionSource, GoldenSourceStore, GovvieSource, InstrumentMaster, PositionSink, Provenance,
    RefDataSource, SinkError, SourceRef, StoredCorpAction, lifecycle,
};
use tonic::{Request, Response, Status};

use crate::readiness::ReadinessGate;
use crate::services::rates_book::RatesPositionStore;
use crate::services::sessions::SessionRegistry;

/// The org cell the CA-realisation leg books into. Corporate actions act on the desk's
/// proprietary inventory as a whole; the movement is stamped to a fixed reference-data
/// entity/book (the limit tree is empty on this path, so the exact cell only names the leg).
const CA_ENTITY: u32 = 0;
/// The netting book the CA-realisation leg books into (see [`CA_ENTITY`]).
const CA_BOOK: u32 = 0;

/// The reference-data capability a CA confirm/apply requires (fixed income today).
const REFDATA_FI: Capability = Capability::new(Action::Refdata, AssetClass::FixedIncome);

/// Format a civil date as ISO `YYYY-MM-DD` (the wire form).
fn civil_to_iso(d: CivilDate) -> String {
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

/// Map the corporate-actions CAEV onto the wire enum tag.
fn caev_to_wire(c: Caev) -> CorpEventType {
    match c {
        Caev::Redm => CorpEventType::Redm,
        Caev::Intr => CorpEventType::Intr,
        Caev::Mcal => CorpEventType::Mcal,
        Caev::Pcal => CorpEventType::Pcal,
        Caev::Pred => CorpEventType::Pred,
        Caev::Draw => CorpEventType::Draw,
        Caev::Bput => CorpEventType::Bput,
        Caev::Tend => CorpEventType::Tend,
        Caev::Exof => CorpEventType::Exof,
        Caev::Conv => CorpEventType::Conv,
    }
}

/// Map the CAMV mandatory/voluntary indicator onto the wire enum tag.
fn camv_to_wire(c: Camv) -> CorpMandatory {
    match c {
        Camv::Mand => CorpMandatory::Mand,
        Camv::Volu => CorpMandatory::Volu,
        Camv::Chos => CorpMandatory::Chos,
    }
}

/// Map the lifecycle status onto the wire enum tag.
fn status_to_wire(s: CaStatus) -> CorpActionStatus {
    match s {
        CaStatus::Announced => CorpActionStatus::Announced,
        CaStatus::Elected => CorpActionStatus::Elected,
        CaStatus::Confirmed => CorpActionStatus::Confirmed,
        CaStatus::Applied => CorpActionStatus::Applied,
        CaStatus::Reversed => CorpActionStatus::Reversed,
        CaStatus::Cancelled => CorpActionStatus::Cancelled,
    }
}

/// Project a stored corporate action onto its wire descriptor.
fn stored_ca_to_wire(ca: &StoredCorpAction) -> CorporateActionDesc {
    let ev = &ca.event;
    CorporateActionDesc {
        ca_id: ca.ca_id.clone(),
        isin: ev.isin.clone(),
        caev: caev_to_wire(ev.caev) as i32,
        camv: camv_to_wire(ev.camv) as i32,
        status: status_to_wire(ev.status) as i32,
        announcement_date: civil_to_iso(ev.dates.announcement),
        record_date: civil_to_iso(ev.dates.record),
        ex_date: civil_to_iso(ev.dates.ex),
        response_deadline: ev.dates.response_deadline.map(civil_to_iso),
        payment_date: civil_to_iso(ev.dates.payment),
        cash_per_100: ev.terms.cash_per_100,
        redeemed_fraction: ev.terms.redeemed_fraction,
        target_instrument: ev.terms.target_instrument.clone(),
        target_units_per_100: ev.terms.target_units_per_100,
        source_ref: ev.source_ref.clone(),
        source_priority: u32::from(ca.provenance.source_priority),
        source: ca.provenance.source.0.clone(),
    }
}

/// Project a master's effective schedule onto the wire schedule flows + pool factor.
fn schedule_to_wire(master: &InstrumentMaster) -> (Vec<InstrumentScheduleFlow>, f64) {
    let flows = master
        .schedule
        .flows()
        .iter()
        .map(|f| InstrumentScheduleFlow {
            date: civil_to_iso(f.date),
            coupon: f.coupon,
            principal: f.principal,
        })
        .collect();
    (flows, master.schedule.pool_factor())
}

/// Map a master's coupons-per-year onto the linear-rates payment frequency (used only to
/// name the CA-realisation leg; the frequency is not economically load-bearing on a realise).
fn freq_to_wire(coupons_per_year: u32) -> PaymentFrequency {
    match coupons_per_year {
        4 => PaymentFrequency::Quarterly,
        2 => PaymentFrequency::SemiAnnual,
        _ => PaymentFrequency::Annual,
    }
}

/// A [`PositionSink`] that captures the concrete [`PositionDelta`] the lifecycle computes,
/// without itself holding a store. The rates-book leg is booked by the edge (before the
/// golden-store mutation) so a limit breach aborts the apply atomically.
#[derive(Default)]
struct CaptureSink {
    delta: Option<PositionDelta>,
}

impl PositionSink for CaptureSink {
    fn apply_delta(
        &mut self,
        _instrument_id: &str,
        delta: &PositionDelta,
    ) -> Result<(), SinkError> {
        if !(delta.face_delta.is_finite() && delta.cash.is_finite()) {
            return Err(SinkError::NonFiniteDelta);
        }
        self.delta = Some(delta.clone());
        Ok(())
    }
}

/// The corporate-actions service edge (shared behind an `Arc` by the gRPC server and the WS
/// mirror, like every other edge). The golden store is behind a `Mutex` — its writes are `&mut`
/// and rare (control-plane), never a latency budget.
pub struct CorporateActionsEdge {
    sessions: Arc<SessionRegistry>,
    gate: Arc<ReadinessGate>,
    rates: Arc<RatesPositionStore>,
    store: Mutex<GoldenSourceStore>,
}

impl CorporateActionsEdge {
    /// Construct the edge over an already-opened golden store and the shared rates book.
    #[must_use]
    pub fn new(
        sessions: Arc<SessionRegistry>,
        gate: Arc<ReadinessGate>,
        rates: Arc<RatesPositionStore>,
        store: GoldenSourceStore,
    ) -> Self {
        Self {
            sessions,
            gate,
            rates,
            store: Mutex::new(store),
        }
    }

    /// Idempotently seed the golden store from a deterministic OSS govvie source: on an
    /// **empty** store, append every derived master and announce, **per instrument, the single
    /// earliest upcoming** schedule-driven MAND event (the next coupon or the redemption) — the
    /// pending action a CA inbox shows — all effective from `recorded_at`. Bounding the announce
    /// to one-per-instrument keeps boot durable-append cost O(instruments) rather than
    /// O(all-future-coupons). A no-op once any instrument exists (an already-populated journal is
    /// left untouched, so a restart never re-appends).
    ///
    /// # Errors
    /// A store append or a source normalization failure.
    pub fn seed_from_source(
        &self,
        source: &GovvieSource,
        recorded_at: CivilDate,
    ) -> Result<(), String> {
        let mut store = self.lock();
        if !store.instrument_ids().is_empty() {
            return Ok(());
        }
        for master in source.masters(recorded_at).map_err(|e| e.to_string())? {
            store.upsert_instrument(master).map_err(|e| e.to_string())?;
        }
        // Keep only the earliest upcoming event per ISIN (source events are ascending per
        // instrument, so the first seen per ISIN is the earliest).
        let mut seen: Vec<String> = Vec::new();
        for ca in source
            .corp_actions(recorded_at)
            .map_err(|e| e.to_string())?
        {
            if seen.iter().any(|i| i == &ca.event.isin) {
                continue;
            }
            seen.push(ca.event.isin.clone());
            lifecycle::announce(&mut store, ca).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Append a new effective-dated instrument-master version — the seam a reference-data
    /// ingestion adapter (or a test) drives to master an instrument the deterministic govvie
    /// source does not itself cover.
    ///
    /// # Errors
    /// A durable-write failure.
    pub fn upsert_master(&self, master: InstrumentMaster) -> Result<(), String> {
        self.lock()
            .upsert_instrument(master)
            .map_err(|e| e.to_string())
    }

    /// Announce a single normalized corporate action (the first append, status `Announced`) —
    /// the seam a customer's vendor-CA ingestion adapter (MT 564 / seev.031) drives, and the
    /// test-injection point for a specific event (e.g. a full call the deterministic govvie
    /// source does not itself derive).
    ///
    /// # Errors
    /// A durable-write failure.
    pub fn announce(&self, ca: StoredCorpAction) -> Result<(), String> {
        let mut store = self.lock();
        lifecycle::announce(&mut store, ca).map_err(|e| e.to_string())
    }

    /// Lock the golden store. The mutex only guards rare control-plane mutation.
    fn lock(&self) -> std::sync::MutexGuard<'_, GoldenSourceStore> {
        self.store
            .lock()
            .expect("corp-actions golden store poisoned")
    }

    /// Require a valid session (the `view` read floor).
    fn require_session(&self, token: &str) -> Result<(), Status> {
        self.sessions
            .validate(token)
            .map(|_| ())
            .ok_or_else(|| Status::unauthenticated("invalid or expired session token"))
    }

    /// Require a valid session that holds the `refdata` capability (FI) — the CA
    /// confirm/apply authority. Deny-wins over any grant.
    fn require_refdata(&self, token: &str) -> Result<(), Status> {
        let who = self
            .sessions
            .validate(token)
            .ok_or_else(|| Status::unauthenticated("invalid or expired session token"))?;
        if who.capabilities().allows(REFDATA_FI) {
            Ok(())
        } else {
            Err(Status::permission_denied(
                "capability refdata·fixed_income required",
            ))
        }
    }

    /// Whether the edge is serving (not draining / still starting).
    fn require_ready(&self) -> Result<(), Status> {
        if self.gate.is_ready() {
            Ok(())
        } else {
            Err(Status::unavailable("server not ready"))
        }
    }

    /// Book the CA position movement as a realising bond leg into the shared rates book:
    /// a face reduction (`face_delta < 0`) books an offsetting `SIDE_SELL` leg, a face
    /// increase (a reversal restoring nominal) a `SIDE_BUY` leg, each sized `|face_delta|`.
    /// An income (coupon) event carries `face_delta == 0` → no leg booked, only cash. The
    /// leg round-trips through the exact [`RatesPositionStore::book`] pre-trade gate a trade
    /// uses; a hard-limit breach surfaces as the caller's `failed_precondition`.
    fn book_realisation(
        &self,
        master: &InstrumentMaster,
        delta: &PositionDelta,
    ) -> Result<(), Status> {
        if delta.face_delta == 0.0 {
            return Ok(());
        }
        let side = if delta.face_delta < 0.0 {
            Side::Sell
        } else {
            Side::Buy
        };
        let m = &master.terms.maturity_date;
        let bond = BondInstrument {
            coupon_rate: master.terms.coupon_rate,
            coupon_frequency: freq_to_wire(master.terms.coupons_per_year) as i32,
            day_count: celnet_proto::AccrualBasis::Act365Fixed as i32,
            maturity_date: Some(BrokenDate {
                year: m.year,
                month: u32::from(m.month),
                day: u32::from(m.day),
            }),
            redemption: delta.face_delta.abs(),
            side: side as i32,
            // Corporate-action rebooking preserves the security's identity via its own
            // instrument records; the wire identity fields default empty here.
            ..Default::default()
        };
        let pos = RatesPosition {
            position_id: 0,
            entity: CA_ENTITY,
            book: CA_BOOK,
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Bond(bond)),
            }),
            ..Default::default()
        };
        self.rates.book(pos).map(|_| ())
    }
}

#[tonic::async_trait]
impl CorporateActionsService for CorporateActionsEdge {
    async fn list_instrument_schedule(
        &self,
        request: Request<ListInstrumentScheduleRequest>,
    ) -> Result<Response<ListInstrumentScheduleResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_session(&req.session_token)?;

        let store = self.lock();
        let master = store.latest_instrument(&req.instrument_id).ok_or_else(|| {
            Status::not_found(format!("unknown instrument {}", req.instrument_id))
        })?;
        let (flows, pool_factor) = schedule_to_wire(master);
        Ok(Response::new(ListInstrumentScheduleResponse {
            instrument_id: req.instrument_id,
            flows,
            pool_factor,
            correlation_id: req.correlation_id,
        }))
    }

    async fn list_corporate_actions(
        &self,
        request: Request<ListCorporateActionsRequest>,
    ) -> Result<Response<ListCorporateActionsResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_session(&req.session_token)?;

        let store = self.lock();
        let actions = match req.isin.as_deref() {
            Some(isin) => store.corp_actions_for_isin(isin),
            None => store.current_corp_actions(),
        }
        .iter()
        .map(|c| stored_ca_to_wire(c))
        .collect();
        Ok(Response::new(ListCorporateActionsResponse {
            actions,
            correlation_id: req.correlation_id,
        }))
    }

    async fn confirm_corporate_action(
        &self,
        request: Request<ConfirmCorporateActionRequest>,
    ) -> Result<Response<ConfirmCorporateActionResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_refdata(&req.session_token)?;

        let mut store = self.lock();
        let current = store
            .latest_corp_action(&req.ca_id)
            .ok_or_else(|| Status::not_found(format!("unknown corporate action {}", req.ca_id)))?;
        let effective = current.event.dates.payment;
        let provenance = Provenance {
            source_priority: current.provenance.source_priority,
            source: SourceRef("ca_confirm".to_string()),
            valid_from: effective,
            recorded_at: effective,
            quality: current.provenance.quality,
        };
        lifecycle::confirm(&mut store, &req.ca_id, provenance)
            .map_err(|e| Status::failed_precondition(e.to_string()))?;
        let action = store
            .latest_corp_action(&req.ca_id)
            .map(stored_ca_to_wire)
            .expect("just-confirmed corporate action present");
        Ok(Response::new(ConfirmCorporateActionResponse {
            action: Some(action),
            correlation_id: req.correlation_id,
        }))
    }

    async fn apply_corporate_action(
        &self,
        request: Request<ApplyCorporateActionRequest>,
    ) -> Result<Response<ApplyCorporateActionResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_refdata(&req.session_token)?;

        let mut store = self.lock();
        let ca = store
            .latest_corp_action(&req.ca_id)
            .ok_or_else(|| Status::not_found(format!("unknown corporate action {}", req.ca_id)))?
            .clone();
        if ca.event.status != CaStatus::Confirmed {
            return Err(Status::failed_precondition(format!(
                "corporate action {} is not confirmed (status {:?})",
                req.ca_id, ca.event.status
            )));
        }
        let master = store
            .latest_instrument_by_isin(&ca.event.isin)
            .ok_or_else(|| Status::not_found(format!("no instrument for ISIN {}", ca.event.isin)))?
            .clone();

        // Compute the concrete effect + delta against the CURRENT schedule (pure oracle math),
        // then book the realising leg FIRST so a hard-limit breach aborts BEFORE any golden-store
        // mutation — atomic apply. The lifecycle re-runs the same pure `apply_event` internally.
        let applied = apply_event(&master.schedule, &ca.event)
            .map_err(|e| Status::internal(e.to_string()))?;
        let delta = position_delta(&applied.effect, req.held_face);
        self.book_realisation(&master, &delta)?;

        // Drive the lifecycle: append the post-event schedule version (pricing/DV01 re-derive off
        // it) and supersede the CA as Applied. The CaptureSink records the delta (already booked).
        let effective = ca.event.dates.payment;
        let mut sink = CaptureSink::default();
        let outcome = lifecycle::apply(
            &mut store,
            &mut sink,
            &req.ca_id,
            req.held_face,
            effective,
            effective,
        )
        .map_err(|e| Status::failed_precondition(e.to_string()))?;
        let action = store
            .latest_corp_action(&req.ca_id)
            .map(stored_ca_to_wire)
            .expect("just-applied corporate action present");

        let remaining_flows = u32::try_from(outcome.remaining_flows).unwrap_or(u32::MAX);
        Ok(Response::new(ApplyCorporateActionResponse {
            instrument_id: outcome.instrument_id,
            face_delta: outcome.delta.face_delta,
            cash: outcome.delta.cash,
            remaining_flows,
            action: Some(action),
            correlation_id: req.correlation_id,
        }))
    }
}

#[cfg(test)]
mod tests;
