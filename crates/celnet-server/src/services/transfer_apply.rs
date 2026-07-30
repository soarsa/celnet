//! The **risk-transfer apply engine** — the behavioral core that moves *existing*
//! risk between risk books (`docs/RISK-TRANSFER-REQUIREMENTS.md` §6). It is the
//! server-side complement to the pure [`celnet_risk_transfer`] leaf crate: that
//! crate owns the deterministic numerics (validation, the two-leg computation, the
//! identity plan) with **no** server deps; this module owns all the store
//! interaction that realises those numerics against the live FX
//! [`PositionStore`](crate::services::risk::store::PositionStore) and rates
//! [`RatesPositionStore`](crate::services::rates_book::RatesPositionStore) books.
//!
//! # Two mechanisms (locks the taxonomy, §6.1–§6.2)
//!
//! * **Re-attribution** ([`TransferApplier::apply_reattribute`], §6.1) — a pure
//!   re-stamp of the routing dimension: the position(s) are re-labelled into the
//!   target book (a partial move splits a line), economics unchanged, no new trade,
//!   no P&L crosses. The target book's hard-limit gate (and its ancestors') is
//!   re-checked over a read snapshot **before** any mutation, exactly as a routed
//!   fill is gated.
//! * **Economic transfer** ([`TransferApplier::apply_economic`], §6.2) — two
//!   offsetting **mirror bookings** at a transfer price: an offsetting leg out of
//!   the source (so it net-flattens the moved quantity and realises P&L to the
//!   transfer price) matched by an opening leg into the target (so it opens at that
//!   price). The pair nets to zero at the firm level. The legs are staged
//!   atomically: if the target leg's limit gate refuses, the source leg is rolled
//!   back — **never** a half-transfer.
//!
//! # Off the hot core (guardrail 11)
//!
//! Everything here runs on the async booking tier — the same tier the routed
//! booking sinks already run on — never the pinned zero-alloc pricing thread. Each
//! mutation advances the store's monotonic `risk_version`, so the per-book risk
//! stream re-aggregates lazily on its next tick (no timer thread).
//!
//! # Marks & greeks provenance
//!
//! The mark and greeks a transfer's numerics consume are read straight off the
//! marked position facts (never re-priced here): the FX per-unit mark is the marked
//! option premium (`premium_quote / notional`) and its greeks are the canonical
//! notional-scaled leaf greeks; the rates mark is the instrument's contractual
//! dealt level and its DV01 is the curve-free linear PV01 proxy the limit gate
//! charges (an FX line carries no DV01, a rates line no option greeks — each
//! honestly `0.0`, mirroring how the per-book gate skips the absent measure). See
//! [`PositionStore::fx_transfer_view`] /
//! [`RatesPositionStore::rates_transfer_view`] for the exact derivations.

// `tonic::Status` is the platform-standard edge error carried by every booking-tier
// `Result` (the same house convention as `services::auth` / `services::rates_book`): a
// large `Err` variant is accepted rather than boxed per call.
#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::Arc;

use celnet_proto::{AttributionRecord, BookId, Owner, owner};
use celnet_risk_transfer::{
    BookRef, MoveKind, MovedRisk, PositionRef, PositionSlice, PriceBasis, RiskVector,
    TransferContext, TransferLegs, TransferPrice, TransferQuantity, compute_legs,
    plan_position_moves,
};

use crate::services::rates_book::RatesPositionStore;
use crate::services::risk::store::{BookedPosition, PositionStore};

/// Which asset book a transfer operates over. A single transfer is homogeneous —
/// its positions all live in the FX book or all in the rates book (a mixed
/// selection is rejected by [`TransferApplier::classify`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferAsset {
    /// The FX (vanilla / exotic) warehouse.
    Fx,
    /// The linear-rates / bond book.
    Rates,
}

/// The apply engine. Owns shared handles to both position books and realises a
/// validated transfer (validated by the pure [`celnet_risk_transfer::check_transfer`]
/// upstream) against them: reading the marked slices, computing the legs / identity
/// plan through the leaf crate, and staging the store mutations with the correct
/// limit gates and atomic rollback. The orchestrator layer (a peer module) owns the
/// lifecycle, four-eyes, provenance assembly, and wire mapping and calls exactly the
/// public methods below.
#[derive(Debug, Clone)]
pub struct TransferApplier {
    store: Arc<PositionStore>,
    rates: Arc<RatesPositionStore>,
}

impl TransferApplier {
    /// Build an applier over the shared FX and rates position books.
    #[must_use]
    pub fn new(store: Arc<PositionStore>, rates: Arc<RatesPositionStore>) -> Self {
        Self { store, rates }
    }

    /// Classify the selected positions as an FX or rates transfer by probing both
    /// books: a position present in exactly one book fixes the asset; every selected
    /// position must agree.
    ///
    /// # Errors
    /// `invalid_argument` for an empty selection; `not_found` if a position is booked
    /// in neither book; `failed_precondition` if a position is in both books or the
    /// selection spans both.
    pub fn classify(
        &self,
        source_book: &str,
        position_ids: &[u64],
    ) -> Result<TransferAsset, tonic::Status> {
        let _ = source_book; // membership is validated by `check_transfer`; classify fixes asset.
        if position_ids.is_empty() {
            return Err(tonic::Status::invalid_argument(
                "a transfer selects at least one position",
            ));
        }
        let mut asset: Option<TransferAsset> = None;
        for &pid in position_ids {
            let in_fx = self.store.fx_transfer_view(pid).is_some();
            let in_rates = self.rates.rates_transfer_view(pid).is_some();
            let this = match (in_fx, in_rates) {
                (true, false) => TransferAsset::Fx,
                (false, true) => TransferAsset::Rates,
                (true, true) => {
                    return Err(tonic::Status::failed_precondition(format!(
                        "position {pid} exists in both the FX and rates books"
                    )));
                }
                (false, false) => {
                    return Err(tonic::Status::not_found(format!(
                        "position {pid} is not booked in either book"
                    )));
                }
            };
            match asset {
                None => asset = Some(this),
                Some(a) if a == this => {}
                Some(_) => {
                    return Err(tonic::Status::failed_precondition(
                        "the selected positions span both the FX and rates books",
                    ));
                }
            }
        }
        asset.ok_or_else(|| {
            tonic::Status::invalid_argument("a transfer selects at least one position")
        })
    }

    /// Build the [`TransferContext`] the pure [`celnet_risk_transfer::check_transfer`]
    /// validates against: every selected position's current book + signed notional from
    /// the live store, plus the caller-supplied `books` registry snapshot. A position id
    /// the store does not hold is simply absent from the map — `check_transfer` then
    /// surfaces it as `PositionNotFound`.
    #[must_use]
    pub fn build_context(
        &self,
        books: &HashMap<String, BookRef>,
        source_book: &str,
        position_ids: &[u64],
        asset: TransferAsset,
    ) -> TransferContext {
        let _ = source_book;
        let mut positions: HashMap<u64, PositionRef> = HashMap::new();
        for &pid in position_ids {
            let pref = match asset {
                TransferAsset::Fx => self.store.fx_transfer_view(pid).map(|v| PositionRef {
                    risk_book_id: v.risk_book.unwrap_or_default(),
                    signed_notional: v.signed_notional,
                }),
                TransferAsset::Rates => self.rates.rates_transfer_view(pid).map(|v| PositionRef {
                    risk_book_id: v.risk_book.unwrap_or_default(),
                    signed_notional: v.signed_notional,
                }),
            };
            if let Some(pref) = pref {
                positions.insert(pid, pref);
            }
        }
        TransferContext {
            positions,
            books: books.clone(),
        }
    }

    /// Resolve the numeric transfer price + its recorded basis (§3.3). `Agreed(x)` books
    /// at `x`; `Mid` / `MarkToMarket` resolve to the **notional-weighted average mark** of
    /// the selected slices — a mark-to-market internal cross, so a transfer at this price
    /// realises zero P&L in the source (each slice crosses at its own fair value) and the
    /// P&L only moves when an `Agreed` override departs from the mark.
    ///
    /// # Errors
    /// Propagates a slice read error (a position missing or not in `source_book`).
    pub fn resolve_price(
        &self,
        price: TransferPrice,
        source_book: &str,
        position_ids: &[u64],
        asset: TransferAsset,
    ) -> Result<(f64, PriceBasis), tonic::Status> {
        if let TransferPrice::Agreed(x) = price {
            return Ok((x, PriceBasis::Agreed));
        }
        let slices = match asset {
            TransferAsset::Fx => self.read_fx_slices(source_book, position_ids)?,
            TransferAsset::Rates => self.read_rates_slices(source_book, position_ids)?,
        };
        let mut weight_sum = 0.0f64;
        let mut mark_sum = 0.0f64;
        for s in &slices {
            let w = s.signed_notional.abs();
            weight_sum += w;
            mark_sum += w * s.mark;
        }
        let avg = if weight_sum > 0.0 {
            mark_sum / weight_sum
        } else {
            0.0
        };
        Ok((avg, price.basis()))
    }

    /// **Re-attribution** (§6.1): re-stamp the selected positions into `target_book`
    /// (splitting each line for a partial move), after re-checking the target's (and its
    /// ancestors') hard notional caps over a read snapshot **before** any mutation. No new
    /// trade, no P&L crosses — economics are unchanged. Returns the aggregate
    /// [`MovedRisk`] (for the provenance record; a re-attribution realises no P&L).
    ///
    /// # Errors
    /// Propagates a slice read error; `failed_precondition` if the target (or an ancestor)
    /// would breach a hard cap — in which case **nothing** is moved (the store is left
    /// unmutated).
    pub fn apply_reattribute(
        &self,
        source_book: &str,
        target_book: &str,
        position_ids: &[u64],
        quantity: TransferQuantity,
        asset: TransferAsset,
    ) -> Result<MovedRisk, tonic::Status> {
        match asset {
            TransferAsset::Fx => {
                let slices = self.read_fx_slices(source_book, position_ids)?;
                let plan = plan_position_moves(&slices, quantity, || self.store.mint_position_id());
                let (net, gross) = incoming_from_plan(&plan, &slices);
                // Gate the target BEFORE any mutation — a breach leaves the store unmutated.
                self.store
                    .check_risk_book_headroom(target_book, net, gross, position_ids)?;
                for mv in &plan {
                    match mv.kind {
                        MoveKind::Whole => {
                            self.store
                                .restamp_risk_book(mv.source_position_id, target_book)?;
                        }
                        MoveKind::Split {
                            remainder_notional,
                            moved_notional,
                            moved_position_id,
                        } => {
                            self.store.split_position(
                                mv.source_position_id,
                                remainder_notional,
                                moved_position_id,
                                moved_notional,
                                target_book,
                            )?;
                        }
                    }
                }
                Ok(compute_legs(&slices, 0.0, quantity).moved_risk)
            }
            TransferAsset::Rates => {
                let slices = self.read_rates_slices(source_book, position_ids)?;
                let plan = plan_position_moves(&slices, quantity, || self.rates.mint_position_id());
                let (net, gross) = incoming_from_plan(&plan, &slices);
                self.rates
                    .check_risk_book_headroom(target_book, net, gross, position_ids)?;
                for mv in &plan {
                    match mv.kind {
                        MoveKind::Whole => {
                            self.rates
                                .restamp_risk_book(mv.source_position_id, target_book)?;
                        }
                        MoveKind::Split {
                            remainder_notional,
                            moved_notional,
                            moved_position_id,
                        } => {
                            self.rates.split_rates_position(
                                mv.source_position_id,
                                remainder_notional,
                                moved_position_id,
                                moved_notional,
                                target_book,
                            )?;
                        }
                    }
                }
                Ok(compute_legs(&slices, 0.0, quantity).moved_risk)
            }
        }
    }

    /// **Economic transfer** (§6.2): stage the two offsetting mirror bookings at
    /// `resolved_price` — an offsetting leg out of `source_book` (opposite side, so the
    /// source net-flattens the moved quantity and realises P&L to the price) and an
    /// opening leg into `target_book` (same side, opening at the price) — per selected
    /// slice, so a heterogeneous selection books each instrument's own economics (the
    /// aggregate still nets to zero at the firm level). Applied **atomically**: if any leg
    /// (the target's limit gate, typically) refuses, every already-booked leg is rolled
    /// back — never a half-transfer. Returns the aggregate [`TransferLegs`] (carrying the
    /// realised source P&L + moved risk) for the provenance record.
    ///
    /// The desk/trader arguments build the two legs' [`AttributionRecord`]s (org book +
    /// trader seat); the desk itself resolves from the book hierarchy at the FX booking
    /// seam and is threaded for the upstream provenance record, not the FactKey.
    ///
    /// # Errors
    /// Propagates a slice read error; `failed_precondition` on any leg's hard-limit breach
    /// (with the paired legs rolled back).
    #[allow(clippy::too_many_arguments)]
    pub fn apply_economic(
        &self,
        source_book: &str,
        target_book: &str,
        source_desk: &str,
        source_trader: &str,
        target_desk: &str,
        target_trader: &str,
        position_ids: &[u64],
        quantity: TransferQuantity,
        resolved_price: f64,
        asset: TransferAsset,
    ) -> Result<TransferLegs, tonic::Status> {
        let _ = (source_desk, target_desk); // desk resolves from the book hierarchy at booking.
        match asset {
            TransferAsset::Fx => {
                let views = self.fx_views(source_book, position_ids)?;
                let slices: Vec<PositionSlice> = views.iter().map(fx_slice).collect();
                let legs = compute_legs(&slices, resolved_price, quantity);
                let f = moved_fraction(&slices, quantity);
                let source_attr = transfer_attribution(source_book, source_trader);
                let target_attr = transfer_attribution(target_book, target_trader);
                // Stage per-slice offsetting + opening legs, tracking every booked id so a
                // refused leg rolls the whole pair back (never a half-transfer).
                let mut booked: Vec<u64> = Vec::with_capacity(views.len() * 2);
                for view in &views {
                    let moved_i = view.signed_notional * f;
                    let source_leg = BookedPosition {
                        position_id: self.store.mint_position_id(),
                        notional_base: -moved_i,
                        ..view.booked
                    };
                    if let Err(e) =
                        self.store
                            .book_into_risk_book(source_leg, &source_attr, source_book)
                    {
                        self.rollback_fx(&booked);
                        return Err(e);
                    }
                    booked.push(source_leg.position_id);
                    let target_leg = BookedPosition {
                        position_id: self.store.mint_position_id(),
                        notional_base: moved_i,
                        ..view.booked
                    };
                    if let Err(e) =
                        self.store
                            .book_into_risk_book(target_leg, &target_attr, target_book)
                    {
                        self.rollback_fx(&booked);
                        return Err(e);
                    }
                    booked.push(target_leg.position_id);
                }
                Ok(legs)
            }
            TransferAsset::Rates => {
                let _ = (source_trader, target_trader); // a rates cell carries no trader seat.
                let views = self.rates_views(source_book, position_ids)?;
                let slices: Vec<PositionSlice> = views.iter().map(rates_slice).collect();
                let legs = compute_legs(&slices, resolved_price, quantity);
                let f = moved_fraction(&slices, quantity);
                let mut booked: Vec<u64> = Vec::with_capacity(views.len() * 2);
                for view in &views {
                    let moved_i = view.signed_notional * f;
                    let source_leg = crate::services::rates_book::rates_with_signed_notional(
                        &view.template,
                        -moved_i,
                        0,
                    );
                    match self.rates.book_into_risk_book(source_leg, source_book) {
                        Ok(p) => booked.push(p.position_id),
                        Err(e) => {
                            self.rollback_rates(&booked);
                            return Err(e);
                        }
                    }
                    let target_leg = crate::services::rates_book::rates_with_signed_notional(
                        &view.template,
                        moved_i,
                        0,
                    );
                    match self.rates.book_into_risk_book(target_leg, target_book) {
                        Ok(p) => booked.push(p.position_id),
                        Err(e) => {
                            self.rollback_rates(&booked);
                            return Err(e);
                        }
                    }
                }
                Ok(legs)
            }
        }
    }

    // ---- internal helpers -------------------------------------------------------

    /// Read the marked FX views for the selected positions, confirming each is currently
    /// in `source_book`. Returns them in the given order.
    fn fx_views(
        &self,
        source_book: &str,
        position_ids: &[u64],
    ) -> Result<Vec<crate::services::risk::store::FxTransferView>, tonic::Status> {
        let mut out = Vec::with_capacity(position_ids.len());
        for &pid in position_ids {
            let view = self.store.fx_transfer_view(pid).ok_or_else(|| {
                tonic::Status::not_found(format!("FX position {pid} is not booked"))
            })?;
            if view.risk_book.as_deref() != Some(source_book) {
                return Err(tonic::Status::failed_precondition(format!(
                    "position {pid} is not in source book {source_book} (it is in {})",
                    view.risk_book.as_deref().unwrap_or("<unrouted>")
                )));
            }
            out.push(view);
        }
        Ok(out)
    }

    /// Read the marked rates views for the selected positions, confirming each is
    /// currently in `source_book`. Returns them in the given order.
    fn rates_views(
        &self,
        source_book: &str,
        position_ids: &[u64],
    ) -> Result<Vec<crate::services::rates_book::RatesTransferView>, tonic::Status> {
        let mut out = Vec::with_capacity(position_ids.len());
        for &pid in position_ids {
            let view = self.rates.rates_transfer_view(pid).ok_or_else(|| {
                tonic::Status::not_found(format!("rates position {pid} is not booked"))
            })?;
            if view.risk_book.as_deref() != Some(source_book) {
                return Err(tonic::Status::failed_precondition(format!(
                    "position {pid} is not in source book {source_book} (it is in {})",
                    view.risk_book.as_deref().unwrap_or("<unrouted>")
                )));
            }
            out.push(view);
        }
        Ok(out)
    }

    /// The marked FX slices (id, signed notional, per-unit premium mark, canonical greeks)
    /// for the selected positions — the pure numerics' input.
    fn read_fx_slices(
        &self,
        source_book: &str,
        position_ids: &[u64],
    ) -> Result<Vec<PositionSlice>, tonic::Status> {
        Ok(self
            .fx_views(source_book, position_ids)?
            .iter()
            .map(fx_slice)
            .collect())
    }

    /// The marked rates slices (id, signed notional, dealt-level mark, linear DV01) for
    /// the selected positions.
    fn read_rates_slices(
        &self,
        source_book: &str,
        position_ids: &[u64],
    ) -> Result<Vec<PositionSlice>, tonic::Status> {
        Ok(self
            .rates_views(source_book, position_ids)?
            .iter()
            .map(rates_slice)
            .collect())
    }

    /// Un-book every staged FX leg (economic-transfer rollback).
    fn rollback_fx(&self, booked: &[u64]) {
        for &id in booked {
            self.store.remove_position(id);
        }
    }

    /// Un-book every staged rates leg (economic-transfer rollback).
    fn rollback_rates(&self, booked: &[u64]) {
        for &id in booked {
            self.rates.remove_position(id);
        }
    }
}

/// The moved fraction of each slice: `1.0` for `Full`, `q / |Σ signed_notional|` for
/// `Partial(q)` — the same fraction the pure [`compute_legs`] applies, so the per-slice
/// legs staged here sum to the aggregate [`TransferLegs`] returned. Guards a zero
/// aggregate to `0.0`.
fn moved_fraction(slices: &[PositionSlice], quantity: TransferQuantity) -> f64 {
    match quantity {
        TransferQuantity::Full => 1.0,
        TransferQuantity::Partial(q) => {
            let agg_abs: f64 = slices.iter().map(|s| s.signed_notional).sum::<f64>().abs();
            if agg_abs > 0.0 { q / agg_abs } else { 0.0 }
        }
    }
}

/// The incoming `(net, gross)` notional a re-attribution plan moves into the target — the
/// sum of each move's moved notional (a whole move moves the slice's full signed notional;
/// a split moves its `moved_notional`). Used to gate the target book's caps up front.
fn incoming_from_plan(
    plan: &[celnet_risk_transfer::PositionMove],
    slices: &[PositionSlice],
) -> (f64, f64) {
    let mut net = 0.0f64;
    let mut gross = 0.0f64;
    for mv in plan {
        let moved = match mv.kind {
            MoveKind::Whole => slices
                .iter()
                .find(|s| s.position_id == mv.source_position_id)
                .map(|s| s.signed_notional)
                .unwrap_or(0.0),
            MoveKind::Split { moved_notional, .. } => moved_notional,
        };
        net += moved;
        gross += moved.abs();
    }
    (net, gross)
}

/// The pure slice for an FX view: canonical option greeks, no DV01 (FX vanilla carries
/// none — the per-book gate skips `max_dv01` identically).
fn fx_slice(view: &crate::services::risk::store::FxTransferView) -> PositionSlice {
    PositionSlice {
        position_id: view.booked.position_id,
        signed_notional: view.signed_notional,
        mark: view.mark,
        risk: RiskVector {
            dv01: 0.0,
            delta: view.delta,
            gamma: view.gamma,
            vega: view.vega,
            theta: view.theta,
        },
    }
}

/// The pure slice for a rates view: linear DV01, no option greeks (a linear-rates line
/// carries none — each honestly `0.0`).
fn rates_slice(view: &crate::services::rates_book::RatesTransferView) -> PositionSlice {
    PositionSlice {
        position_id: view.template.position_id,
        signed_notional: view.signed_notional,
        mark: view.mark,
        risk: RiskVector {
            dv01: view.dv01,
            delta: 0.0,
            gamma: 0.0,
            vega: 0.0,
            theta: 0.0,
        },
    }
}

/// Build the [`AttributionRecord`] for a transfer leg: its holder book + trader seat (the
/// org placement [`PositionStore::book_into_risk_book`] interns into the FactKey). The
/// explicit risk-book stamp is passed separately to the booking primitive.
fn transfer_attribution(book: &str, trader: &str) -> AttributionRecord {
    AttributionRecord {
        quoted_by: None,
        held_by: Some(BookId {
            book: book.to_owned(),
            owner: Some(Owner {
                seat: Some(owner::Seat::Trader(trader.to_owned())),
            }),
        }),
        won: None,
        lp_count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::identity::RiskLimits;
    use crate::services::risk::store::{BookedPosition, RiskBookLimitDef};
    use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    /// A booked FX vanilla call of `notional` base.
    fn booked(id: u64, notional: f64) -> BookedPosition {
        BookedPosition {
            position_id: id,
            pair: eurusd(),
            option: OptionType::Call,
            notional_base: notional,
            inputs: VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            quoted_delta: DeltaConvention::SpotUnadjusted,
            premium_style: PremiumStyle::DomesticPips,
            surface_version: 1,
        }
    }

    fn attribution(book: &str) -> AttributionRecord {
        transfer_attribution(book, "trader-1")
    }

    /// A `RiskBookLimitDef` for `id` with an optional net-notional cap and no parent.
    fn book_def(id: &str, max_net: Option<f64>) -> RiskBookLimitDef {
        RiskBookLimitDef {
            id: id.to_owned(),
            parent_id: None,
            limits: Some(RiskLimits {
                max_net_notional: max_net,
                max_gross_notional: None,
                max_dv01: None,
            }),
        }
    }

    /// An applier over a fresh FX store (with the given book caps) that has `pos1` booked
    /// into `BOOK-A`.
    fn fx_fixture(
        book_defs: Vec<RiskBookLimitDef>,
        pos1_notional: f64,
    ) -> (TransferApplier, Arc<PositionStore>) {
        let store = Arc::new(PositionStore::new());
        store.set_risk_books(book_defs);
        store
            .book_into_risk_book(booked(1, pos1_notional), &attribution("BOOK-A"), "BOOK-A")
            .expect("seed position books into A");
        let rates = Arc::new(RatesPositionStore::new());
        (TransferApplier::new(store.clone(), rates), store)
    }

    /// Net signed notional a book currently carries (summed over its stamped facts).
    fn book_net(store: &PositionStore, book: &str) -> f64 {
        store
            .positions_in_risk_book(book)
            .iter()
            .map(|f| f.measure.position.notional_base)
            .sum()
    }

    /// A re-attribution re-stamps the position into the target and the target's cap is
    /// consulted: a move that would breach the target cap is REJECTED and leaves the
    /// position unmoved in the source; a headroom-clear move succeeds.
    #[test]
    fn reattribute_restamps_and_rerolls_limits() {
        // BOOK-A uncapped source, BOOK-CAP net cap 5m, BOOK-OK net cap 50m.
        let (applier, store) = fx_fixture(
            vec![
                RiskBookLimitDef {
                    id: "BOOK-A".to_owned(),
                    parent_id: None,
                    limits: None,
                },
                book_def("BOOK-CAP", Some(5.0e6)),
                book_def("BOOK-OK", Some(50.0e6)),
            ],
            10.0e6,
        );

        // Moving the 10m line into the 5m-capped book is refused — and the position is
        // left in the source, unmoved (the load-bearing pre-mutation reject).
        let breached = applier.apply_reattribute(
            "BOOK-A",
            "BOOK-CAP",
            &[1],
            TransferQuantity::Full,
            TransferAsset::Fx,
        );
        assert!(breached.is_err(), "a target-cap breach must reject");
        assert_eq!(store.risk_book_of(1).as_deref(), Some("BOOK-A"));
        assert_eq!(store.positions_in_risk_book("BOOK-CAP").len(), 0);

        // Into the 50m-headroom book it succeeds and the stamp flips.
        let moved = applier
            .apply_reattribute(
                "BOOK-A",
                "BOOK-OK",
                &[1],
                TransferQuantity::Full,
                TransferAsset::Fx,
            )
            .expect("headroom-clear re-attribution books");
        assert_eq!(store.risk_book_of(1).as_deref(), Some("BOOK-OK"));
        assert!(store.positions_in_risk_book("BOOK-A").is_empty());
        assert_eq!(store.positions_in_risk_book("BOOK-OK").len(), 1);
        // A re-attribution moves the whole 10m of risk (its notional) and no P&L.
        assert!((moved.notional_base - 10.0e6).abs() < 1e-6);
    }

    /// An economic transfer whose TARGET leg breaches the target cap rolls BOTH legs back:
    /// the source book is byte-identical to before (no half-transfer).
    #[test]
    fn economic_transfer_atomic_rollback() {
        // Source BOOK-A uncapped; target BOOK-CAP net cap 5m (a 10m open breaches).
        let (applier, store) = fx_fixture(
            vec![
                RiskBookLimitDef {
                    id: "BOOK-A".to_owned(),
                    parent_id: None,
                    limits: None,
                },
                book_def("BOOK-CAP", Some(5.0e6)),
            ],
            10.0e6,
        );
        let a_before = book_net(&store, "BOOK-A");
        let a_count_before = store.positions_in_risk_book("BOOK-A").len();

        let result = applier.apply_economic(
            "BOOK-A",
            "BOOK-CAP",
            "DESK-1",
            "trader-1",
            "DESK-2",
            "trader-2",
            &[1],
            TransferQuantity::Full,
            0.02, // resolved transfer price (per-unit premium) — irrelevant to the breach.
            TransferAsset::Fx,
        );
        assert!(
            result.is_err(),
            "a target-cap breach must reject the transfer"
        );
        // The staged source-offset leg was rolled back: BOOK-A is exactly as before.
        assert_eq!(store.positions_in_risk_book("BOOK-A").len(), a_count_before);
        assert!((book_net(&store, "BOOK-A") - a_before).abs() < 1e-6);
        assert!((book_net(&store, "BOOK-A") - 10.0e6).abs() < 1e-6);
        // The target opened nothing.
        assert_eq!(store.positions_in_risk_book("BOOK-CAP").len(), 0);
    }

    /// A clean economic transfer books the source-offset + target-open legs: the source net
    /// notional drops by the moved quantity, the target rises by it, and the moved pair nets
    /// to zero at the firm level.
    #[test]
    fn economic_transfer_books_both_legs() {
        // Both books uncapped so the pair books cleanly.
        let (applier, store) = fx_fixture(
            vec![
                RiskBookLimitDef {
                    id: "BOOK-A".to_owned(),
                    parent_id: None,
                    limits: None,
                },
                RiskBookLimitDef {
                    id: "BOOK-B".to_owned(),
                    parent_id: None,
                    limits: None,
                },
            ],
            10.0e6,
        );
        let a_before = book_net(&store, "BOOK-A");
        let b_before = book_net(&store, "BOOK-B");

        let legs = applier
            .apply_economic(
                "BOOK-A",
                "BOOK-B",
                "DESK-1",
                "trader-1",
                "DESK-1",
                "trader-2",
                &[1],
                TransferQuantity::Full,
                0.02,
                TransferAsset::Fx,
            )
            .expect("clean economic transfer books both legs");

        let a_after = book_net(&store, "BOOK-A");
        let b_after = book_net(&store, "BOOK-B");
        // Source net drops by the moved 10m (offsetting leg), target rises by 10m.
        assert!(
            (a_after - (a_before - 10.0e6)).abs() < 1e-3,
            "source net {a_after}"
        );
        assert!(
            (b_after - (b_before + 10.0e6)).abs() < 1e-3,
            "target net {b_after}"
        );
        // The moved pair nets to zero at the firm level.
        assert!(((a_after - a_before) + (b_after - b_before)).abs() < 1e-3);
        // The returned legs are exact negations (the mirror pair).
        assert!(
            (legs.source_offset.signed_notional + legs.target_open.signed_notional).abs() < 1e-6
        );
        assert!((legs.target_open.signed_notional - 10.0e6).abs() < 1e-6);
    }

    /// A partial re-attribution splits a line: the remainder keeps the original id, the
    /// moved slice gets a fresh id, and the two notionals sum to the original.
    #[test]
    fn partial_split_keeps_id_on_remainder() {
        let (applier, store) = fx_fixture(
            vec![
                RiskBookLimitDef {
                    id: "BOOK-A".to_owned(),
                    parent_id: None,
                    limits: None,
                },
                RiskBookLimitDef {
                    id: "BOOK-B".to_owned(),
                    parent_id: None,
                    limits: None,
                },
            ],
            10.0e6,
        );

        applier
            .apply_reattribute(
                "BOOK-A",
                "BOOK-B",
                &[1],
                TransferQuantity::Partial(4.0e6),
                TransferAsset::Fx,
            )
            .expect("partial re-attribution splits the line");

        // The remainder keeps id 1 in the source at 6m.
        let a = store.positions_in_risk_book("BOOK-A");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].position_id.0, 1, "remainder keeps the original id");
        let remainder_notional = a[0].measure.position.notional_base;
        assert!(
            (remainder_notional - 6.0e6).abs() < 1e-3,
            "remainder {remainder_notional}"
        );

        // The moved 4m slice is in the target under a FRESH id.
        let b = store.positions_in_risk_book("BOOK-B");
        assert_eq!(b.len(), 1);
        assert_ne!(b[0].position_id.0, 1, "moved slice gets a fresh id");
        let moved_notional = b[0].measure.position.notional_base;
        assert!(
            (moved_notional - 4.0e6).abs() < 1e-3,
            "moved {moved_notional}"
        );

        // The split conserves notional.
        assert!((remainder_notional + moved_notional - 10.0e6).abs() < 1e-3);
    }
}
