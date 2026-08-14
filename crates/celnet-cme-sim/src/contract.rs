//! The **listed Treasury futures** contracts this venue quotes, taken from
//! [`celnet_refdata::treasury_futures_universe`], each anchored to the real cash
//! Treasury curve and streamed as an exchange-convention two-way.
//!
//! # Why the sim must quote these
//!
//! An interest-rate hedge on a corporate position is expressed in a benchmark
//! Treasury future sized by the DV01 ratio. If the platform makes those contracts
//! **tradeable** (the server seeds them into its reference registry) but no LP feed
//! **quotes** them, they never reach an aggregated book, `AggregationHub::best_fill`
//! finds no composite line, and every futures hedge silently backstops to the
//! synthetic COMPOSITE venue. This module closes that gap: everything
//! `celnet-refdata` lists, the sim prices.
//!
//! # How a contract is priced — and what it is anchored to
//!
//! A futures price is **not** a bond price, and it is not fabricated here either.
//! Each contract is priced as the clean price of its **notional deliverable** — the
//! 6%-coupon Treasury the contract's conversion factor is defined against, valued on
//! the first day of the delivery month (see the `celnet_refdata` futures module for
//! the full derivation). Because that security's conversion factor is exactly 1.000
//! by construction, its clean price *is* the contract's price under the standard
//! `F = P_deliverable / CF` identity, with no carry adjustment needed: the notional
//! deliverable is already defined **as of the delivery date**, so there is no
//! spot-to-forward gap to bridge.
//!
//! The yield that security is priced at is taken from the **real cash curve**: the
//! bond in the loaded securities-master snapshot whose maturity sits closest to the
//! notional deliverable's, with its snapshot mid inverted to a yield through
//! [`celnet_bond::yield_to_maturity`] — the same real-leaf inversion the cash feed
//! uses. So a contract's reference level is a real market observation at the matching
//! point of the curve, not a chosen handle, and the futures strip moves consistently
//! with the cash strip the same fleet is quoting.
//!
//! ## What is not modelled
//!
//! The delivery option (the short's switch and timing options) and the identity of
//! the live cheapest-to-deliver security are **not** modelled — the simulated price
//! is the notional-deliverable price, which is the same idealisation the exchange's
//! own conversion factor is built on. A basis trader would see no net-basis dynamics
//! here. That is a deliberate boundary, stated rather than papered over: this is a
//! liquidity simulator, not a basis pricer.
//!
//! # Quotation convention
//!
//! Prices are carried in **points of 100 face**, the same units as the cash bond
//! feed, so the consolidator's absolute divergence tolerance means the same thing on
//! both. They are snapped to the contract's published outright tick — an eighth of a
//! 32nd for the 2-Year, a quarter for the 5-Year, a half for the 10-Year and Ultra
//! 10-Year, a full 32nd for the Bond and Ultra Bond — with the bid rounded down and
//! the offer up, exactly as an exchange-listed market maker quotes.
//! [`celnet_refdata::TreasuryFutureSpec::format_price_32nds`] renders the same price
//! in the `110'165` handle-and-32nds display convention.
//!
//! **Sizes stay in face units**, as they are for the cash feed: an `LpQuote`'s
//! `bid_size` / `offer_size` is a face amount, so a quoted 4,600,000 on a contract of
//! $100,000 face is 46 contracts. That is the one uniform reading available — the
//! quote wire carries no size-unit discriminator — and it is the reading a DV01-ratio
//! hedge already works in, since it converts a cash risk figure to a contract count
//! through the contract's own face value.

use celnet_aggregation::Instrument;
use celnet_refdata::{FutureSpecError, TreasuryFutureSpec};
use celnet_types::{BrokenDate, Ccy, CommodityRef, Symbol, Tenor, Underlying};

use celnet_lp_sim::price::{MidSource, YieldModel};
use celnet_lp_sim::quoted::QuotedLine;
use celnet_lp_sim::universe::TreasuryBond;

/// One listed Treasury futures contract the simulator can price, with the cash-curve
/// yield its notional deliverable is anchored at.
///
/// Built only by [`load_futures_universe`] from a validated
/// [`celnet_refdata::TreasuryFutureSpec`] and a real inverted curve yield, so both
/// fields are known-good.
#[derive(Debug, Clone, PartialEq)]
pub struct FuturesContract {
    /// The contract's full published specification (terms, dates, DV01 derivation).
    pub spec: TreasuryFutureSpec,
    /// The reference yield of the contract's notional deliverable, inverted from the
    /// nearest cash Treasury's real snapshot mid through the analytics leaf.
    pub reference_yield: f64,
}

impl FuturesContract {
    /// The canonical server `instrument_id` — the public contract code (e.g. `ZNZ26`).
    #[must_use]
    pub fn instrument_id(&self) -> &str {
        &self.spec.instrument_id
    }

    /// The aggregation engine's asset-agnostic key for this contract: the contract
    /// code as a vendor-neutral free-form ticker (guardrail #8 — a contract code is an
    /// opaque public code, not a vendor product name) in the contract currency, plus
    /// its last trading date as a broken-date tenor. Injective over the contract code,
    /// and disjoint from every cash bond's key (a CUSIP or curated slug is never a
    /// contract code), so a future and a bond never consolidate onto one line.
    #[must_use]
    pub fn engine_instrument(&self) -> Instrument {
        let expiry = self.spec.last_trading_date;
        Instrument::new(
            Underlying::Commodity(CommodityRef::new(
                Symbol::new(self.spec.instrument_id.clone(), ""),
                Ccy::USD,
            )),
            Tenor::BrokenDate(BrokenDate::new(
                expiry.year,
                u8::try_from(expiry.month).unwrap_or(1),
                u8::try_from(expiry.day).unwrap_or(1),
            )),
        )
    }

    /// The contract's reference price in points of 100 face: the clean price of its
    /// notional deliverable at [`reference_yield`](Self::reference_yield).
    ///
    /// # Errors
    /// Propagates [`FutureSpecError`] if the notional deliverable's schedule is not
    /// buildable (never for a spec from the committed universe).
    pub fn reference_price(&self) -> Result<f64, FutureSpecError> {
        let bond = self.spec.notional_deliverable()?;
        celnet_bond::clean_price(&bond, celnet_types::Rate(self.reference_yield))
            .map_err(FutureSpecError::Schedule)
    }

    /// The contract's DV01 per contract at its own reference (cash-curve) yield —
    /// the denominator of a DV01-ratio hedge sized off this feed's level.
    ///
    /// # Errors
    /// Propagates [`FutureSpecError`] from the derivation.
    pub fn dv01_per_contract(&self) -> Result<f64, FutureSpecError> {
        self.spec.dv01_per_contract(self.reference_yield)
    }

    /// The stochastic mean-reverting-yield model for this contract, seeded at the
    /// cash-curve reference yield: the process reverts to that same yield, jittered by
    /// `perturbation` and pulled at `reversion_per_sec`. The sampled yield is priced
    /// to a clean price through the REAL analytics leaf, so the feed's futures prices
    /// are oracle-anchored exactly as the cash feed's are.
    ///
    /// Returns `None` if the notional deliverable's schedule is not buildable.
    #[must_use]
    pub fn yield_model(&self, reversion_per_sec: f64, perturbation: f64) -> Option<YieldModel> {
        let bond = self.spec.notional_deliverable().ok()?;
        Some(YieldModel {
            bond,
            long_run_yield: self.reference_yield,
            initial_yield: self.reference_yield,
            reversion_per_sec,
            perturbation,
        })
    }

    /// The [`QuotedLine`] this contract streams as, with the listed market's quoting
    /// conventions derived from the contract's own tick and DV01.
    ///
    /// A listed futures panel does not look like a cash-bond panel. Every market
    /// maker shows essentially the same one- or two-tick market and competes on size
    /// and queue position, so the shape is pinned to the contract's minimum price
    /// increment rather than to the fleet's cash defaults:
    ///
    /// * **half-spread → half a tick.** After grid snapping (bid down, offer up) that
    ///   prints a one- or two-tick market, which is how these contracts trade.
    /// * **member lean and starting-yield dispersion → a tenth of a tick each.**
    ///   Members still differ enough to produce a real best-price winner, but the
    ///   total displacement (0.2 tick) stays strictly inside the tightest member's
    ///   half-spread (0.3 tick at the low end of the fleet's spread dispersion), so
    ///   the panel's best bid can never print through its best offer. A crossed
    ///   composite is not cosmetic here: `resolve_rfq_composite` rejects one outright,
    ///   which would starve exactly the hedges this venue exists to fill.
    ///
    /// The dispersion is converted from ticks into **yield** units through the
    /// contract's own derived DV01 (`Δy = Δprice / DV01_per_100 x 1bp`), because the
    /// model is yield-driven and the same yield move is worth wildly different numbers
    /// of ticks on a 2-Year and on an Ultra Bond.
    ///
    /// `base_half_spread` and `base_skew_step` are the fleet's cash-market defaults,
    /// which the returned scales are expressed relative to.
    ///
    /// Returns `None` if the contract has no buildable model or derivable DV01.
    #[must_use]
    pub fn to_line(
        &self,
        base_half_spread: f64,
        base_skew_step: f64,
        reversion_per_sec: f64,
        perturbation: f64,
    ) -> Option<QuotedLine> {
        /// The half-spread a member shows, in ticks.
        const HALF_SPREAD_TICKS: f64 = 0.5;
        /// The peak directional lean of the outermost member, in ticks.
        const LEAN_TICKS: f64 = 0.1;
        /// The peak starting-yield dispersion of a member, in ticks of price.
        const DISPERSION_TICKS: f64 = 0.1;
        /// The panel's outermost member is this many `skew_step`s from the centre
        /// (`(n-1)/2` for the deployed 5-member panel), so a lean budget expressed as
        /// a peak displacement converts to a per-step scale by dividing by it.
        const PANEL_HALF_WIDTH_STEPS: f64 = 2.0;

        let model = self.yield_model(reversion_per_sec, perturbation)?;
        let tick = self.spec.terms.tick_size_points;
        let ratio = |target: f64, base: f64| {
            if base.is_finite() && base > 0.0 {
                target / base
            } else {
                1.0
            }
        };

        // Price displacement -> yield displacement through the contract's own DV01,
        // which is the price move per basis point of the notional deliverable.
        let dv01_per_100 =
            self.spec.dv01_per_contract(self.reference_yield).ok()? / self.spec.point_value();
        if !(dv01_per_100.is_finite() && dv01_per_100 > 0.0) {
            return None;
        }
        let yield_dispersion = DISPERSION_TICKS * tick / dv01_per_100 * 1.0e-4;

        Some(QuotedLine {
            instrument_id: self.spec.instrument_id.clone(),
            display_name: self.spec.name.clone(),
            identity: self.spec.instrument_id.clone(),
            instrument: self.engine_instrument(),
            mid: MidSource::MeanRevertingYield(model),
            spread_scale: ratio(HALF_SPREAD_TICKS * tick, base_half_spread),
            lean_scale: ratio(LEAN_TICKS * tick / PANEL_HALF_WIDTH_STEPS, base_skew_step),
            yield_dispersion: Some(yield_dispersion),
            // No wandering private mark on a listed line: a market maker's view
            // between re-quotes moves by far less than the minimum price increment
            // the two-way is snapped to, so it would round away without reaching the
            // tape. Listed makers differentiate on size and queue position, which
            // this feed carries in the per-member firm size, not on a private mark.
            dealer_view: 0.0,
            tick: Some(tick),
        })
    }
}

/// Load the committed listed Treasury-futures universe, anchoring each contract's
/// notional deliverable to the cash curve implied by `bonds` at `settlement`.
///
/// `bonds` is the loaded government universe (see
/// [`crate::universe::load_government_universe`]); only its **US coupon** securities
/// are used as curve anchors, since the contracts are on US Treasuries. A contract
/// with no usable anchor, or whose notional deliverable's schedule will not build, is
/// dropped — never given a fabricated level.
#[must_use]
pub fn load_futures_universe(
    bonds: &[TreasuryBond],
    settlement: BrokenDate,
) -> Vec<FuturesContract> {
    // The anchor set: US coupon Treasuries whose reference mid inverts to a yield on
    // the real leaf, paired with their maturity. Built once for the whole strip.
    let anchors: Vec<(BrokenDate, f64)> = bonds
        .iter()
        .filter(|b| b.region == "us" && b.security_type.is_coupon_bearing())
        .filter_map(|b| {
            // A zero-width model inverts the reference mid and nothing else — the
            // same real-leaf inversion the cash feed's model uses.
            b.yield_model(settlement, 0.0, 0.0)
                .map(|m| (b.maturity, m.initial_yield))
        })
        .filter(|(_, y)| y.is_finite())
        .collect();

    celnet_refdata::treasury_futures_universe()
        .into_iter()
        .filter_map(|spec| {
            let maturity = spec.notional_deliverable_maturity().ok()?;
            let target = BrokenDate::new(
                maturity.year,
                u8::try_from(maturity.month).ok()?,
                u8::try_from(maturity.day).ok()?,
            );
            let reference_yield = nearest_curve_yield(&anchors, target)?;
            let contract = FuturesContract {
                spec,
                reference_yield,
            };
            // Only admit a contract that actually prices — the quotable set must
            // never advertise an instrument it cannot put a two-way on.
            contract.reference_price().ok()?;
            Some(contract)
        })
        .collect()
}

/// Build the [`QuotedLine`]s for a loaded futures universe (see
/// [`FuturesContract::to_line`]). Contracts with no buildable model are dropped.
#[must_use]
pub fn futures_lines(
    contracts: &[FuturesContract],
    base_half_spread: f64,
    base_skew_step: f64,
    reversion_per_sec: f64,
    perturbation: f64,
) -> Vec<QuotedLine> {
    contracts
        .iter()
        .filter_map(|c| {
            c.to_line(
                base_half_spread,
                base_skew_step,
                reversion_per_sec,
                perturbation,
            )
        })
        .collect()
}

/// The yield of the anchor whose maturity is closest to `target` (ties broken by the
/// earlier maturity, so the choice is deterministic). `None` for an empty anchor set.
fn nearest_curve_yield(anchors: &[(BrokenDate, f64)], target: BrokenDate) -> Option<f64> {
    let key = |d: BrokenDate| i64::from(d.year) * 372 + i64::from(d.month) * 31 + i64::from(d.day);
    let t = key(target);
    anchors
        .iter()
        .min_by_key(|(m, _)| ((key(*m) - t).abs(), key(*m)))
        .map(|(_, y)| *y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_lp_sim::load_government_universe;
    use std::collections::HashSet;

    fn settle() -> BrokenDate {
        BrokenDate::new(2026, 4, 16)
    }

    fn universe() -> Vec<FuturesContract> {
        load_futures_universe(&load_government_universe(false), settle())
    }

    #[test]
    fn every_listed_contract_loads_prices_and_carries_a_real_curve_yield() {
        let contracts = universe();
        assert_eq!(
            contracts.len(),
            celnet_refdata::treasury_futures_universe().len(),
            "every listed contract must be quotable — a tradeable-but-unquotable \
             instrument never reaches an aggregated book"
        );

        for c in &contracts {
            // The anchor yield is a REAL inverted cash-curve yield, in a sane band.
            assert!(
                c.reference_yield.is_finite()
                    && c.reference_yield > 0.0
                    && c.reference_yield < 0.20,
                "{}: implausible anchor yield {}",
                c.instrument_id(),
                c.reference_yield
            );
            // It prices, in a sane band around par for a 6%-coupon deliverable.
            let px = c.reference_price().expect("prices");
            assert!(
                px > 50.0 && px < 200.0,
                "{}: implausible reference price {px}",
                c.instrument_id()
            );
            // And it carries a positive, derived DV01.
            assert!(c.dv01_per_contract().expect("derives") > 0.0);
        }

        // Engine keys are injective over the contract code.
        let keys: HashSet<Instrument> = contracts
            .iter()
            .map(FuturesContract::engine_instrument)
            .collect();
        assert_eq!(keys.len(), contracts.len(), "engine key collision");
    }

    #[test]
    fn futures_keys_never_collide_with_cash_bond_keys() {
        // The union of the tradeable universes must have injective engine keys, or a
        // future and a bond would consolidate onto one composite line.
        let bonds = load_government_universe(false);
        let contracts = load_futures_universe(&bonds, settle());
        let mut keys: HashSet<Instrument> =
            bonds.iter().map(TreasuryBond::engine_instrument).collect();
        let bond_keys = keys.len();
        for c in &contracts {
            assert!(
                keys.insert(c.engine_instrument()),
                "{} collides with a cash bond's engine key",
                c.instrument_id()
            );
        }
        assert_eq!(keys.len(), bond_keys + contracts.len());

        // Same for the canonical wire ids.
        let mut ids: HashSet<&str> = bonds.iter().map(TreasuryBond::instrument_id).collect();
        for c in &contracts {
            assert!(
                ids.insert(c.instrument_id()),
                "{} collides with a cash bond's instrument_id",
                c.instrument_id()
            );
        }
    }

    #[test]
    fn the_anchor_is_the_nearest_cash_maturity() {
        let anchors = [
            (BrokenDate::new(2030, 1, 15), 0.041),
            (BrokenDate::new(2034, 2, 15), 0.045),
            (BrokenDate::new(2056, 5, 15), 0.048),
        ];
        // Closest to the 2034 point.
        let y = nearest_curve_yield(&anchors, BrokenDate::new(2033, 11, 30)).unwrap();
        assert!((y - 0.045).abs() < 1e-12);
        // Closest to the long end.
        let y = nearest_curve_yield(&anchors, BrokenDate::new(2053, 9, 1)).unwrap();
        assert!((y - 0.048).abs() < 1e-12);
        assert!(nearest_curve_yield(&[], BrokenDate::new(2030, 1, 1)).is_none());
    }

    #[test]
    fn a_line_quotes_in_ticks_and_stays_inside_its_own_half_spread() {
        let base_half_spread = 2.0e-2;
        let base_skew_step = 4.0e-3;
        for c in universe() {
            let line = c
                .to_line(base_half_spread, base_skew_step, 0.02, 3.0e-4)
                .unwrap_or_else(|| panic!("no line for {}", c.instrument_id()));
            let tick = c.spec.terms.tick_size_points;
            assert_eq!(line.tick, Some(tick));
            assert_eq!(line.instrument_id, c.spec.instrument_id);
            assert_eq!(line.display_name, c.spec.name);

            // The half-spread lands on half a tick, and the outermost member's lean
            // on a tenth of one.
            let half_spread = base_half_spread * line.spread_scale;
            assert!(
                (half_spread - tick / 2.0).abs() < 1e-12,
                "{}: half-spread is not half a tick",
                c.instrument_id()
            );
            let peak_lean = 2.0 * base_skew_step * line.lean_scale;
            assert!(
                (peak_lean - 0.1 * tick).abs() < 1e-12,
                "{}: peak lean is not a tenth of a tick",
                c.instrument_id()
            );

            // The starting-yield dispersion is a tenth of a tick once converted
            // through the contract's OWN derived DV01 — the whole point of carrying a
            // per-line dispersion rather than the fleet's cash default.
            let dv01_per_100 = c.dv01_per_contract().unwrap() / c.spec.point_value();
            let dispersion_price = line.yield_dispersion.unwrap() * dv01_per_100 / 1.0e-4;
            assert!(
                (dispersion_price - 0.1 * tick).abs() < 1e-9 * tick,
                "{}: dispersion is {dispersion_price} price points, not a tenth of a tick",
                c.instrument_id()
            );

            // The never-crossed budget: total peak member displacement must stay
            // inside the TIGHTEST member's half-spread (0.6x the fleet base).
            let displacement = peak_lean + dispersion_price;
            assert!(
                displacement < 0.6 * half_spread,
                "{}: displacement {displacement} would cross a {half_spread}-wide member",
                c.instrument_id()
            );

            let px = line.mid.mid_at(1_000_000_000, 0.0);
            assert!(
                px.is_finite() && px > 0.0,
                "{}: bad model price {px}",
                line.instrument_id
            );
        }
    }

    #[test]
    fn contracts_are_dropped_rather_than_priced_off_nothing() {
        // With no cash curve to anchor to, no contract is advertised — the sim never
        // invents a level for an instrument it cannot observe.
        assert!(load_futures_universe(&[], settle()).is_empty());
    }
}
