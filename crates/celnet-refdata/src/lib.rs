//! `celnet-refdata` — the single source of truth for CelNet's curated **government-bond
//! static reference data**: the on-the-run government curves the platform ships with,
//! as vendor-neutral, fully-specified [`GovBondSpec`] records.
//!
//! # What this crate is
//!
//! A pure data + integrity crate (no IO beyond an `include_str!` of the committed
//! securities-master snapshot, no wire, no server dependency). It emits one flat
//! universe of government bonds spanning three regions:
//!
//! * **US Treasuries** — parsed from the embedded securities-master snapshot
//!   ([`TREASURY_UNIVERSE_JSON`]); the instrument identity is the CUSIP the LP-SIM
//!   feed already streams, so a registry entry seeded from a spec resolves the exact
//!   `instrument_id` seen on the wire.
//! * **UK gilts** and **EUR govvies** (German Bunds, French OATs, Italian BTPs) —
//!   curated, deterministic on-the-run curves with real market conventions and
//!   ISO-6166 check-valid ISINs ([`curated_universe`]).
//!
//! # Who consumes it
//!
//! * The **server** maps each [`GovBondSpec`] onto a reference-data `InstrumentDef`
//!   (name → the composite/blotter display name; ISIN/CUSIP → external ids;
//!   `region`/`sub_asset_type` → the download taxonomy) and seeds the registry on
//!   boot, so the FI Aggregated Book tiles show a real bond name and the FIX / RFS
//!   security-list download can filter by region and sub-asset-type.
//! * The **LP simulator** prices each spec off the real `celnet_bond` analytics leaf
//!   (never a fabricated handle), so the same universe that names an instrument also
//!   quotes it.
//!
//! Every spec is validated: its ISIN passes the ISO-6166 check digit, its maturity is
//! a real calendar date, and — asserted in this crate's oracle test — every
//! coupon-bearing schedule round-trips price↔yield through the real `celnet_bond`
//! leaf, so the curated data is verified, not merely plausible.

mod curated;
mod isin;
mod model;
mod treasury;

pub use curated::curated_universe;
pub use isin::{build as build_isin, check_digit, is_well_formed};
pub use model::{CivilYmd, GovBondSpec};
pub use treasury::{TREASURY_UNIVERSE_JSON, parse as parse_treasury_universe, treasury_universe};

/// The full curated government-bond universe: every US Treasury from the embedded
/// securities-master snapshot, then the curated UK gilts and EUR govvies, in a stable
/// order (US first, then UK, DE, FR, IT). Deterministic and allocation-only — the
/// same records on every call, so a server seed and an LP-SIM plan agree exactly.
#[must_use]
pub fn government_universe() -> Vec<GovBondSpec> {
    let mut u = treasury_universe();
    u.extend(curated_universe());
    u
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn combined_universe_spans_every_region_and_is_globally_unique() {
        let u = government_universe();
        // US (≥200 from the snapshot) + the 30 curated non-US points.
        assert!(u.len() >= 230, "combined universe too small: {}", u.len());

        for region in ["us", "uk", "de", "fr", "it"] {
            assert!(
                u.iter().any(|s| s.region == region),
                "no {region} bonds in the universe"
            );
        }
        // Every Phase-1 record is a government bond.
        assert!(u.iter().all(|s| s.sub_asset_type == "government"));

        // instrument_id and ISIN are unique across the WHOLE universe — the server
        // keys the registry by instrument_id and cross-refs by ISIN, so a collision
        // would silently merge two securities.
        let ids: HashSet<&str> = u.iter().map(|s| s.instrument_id.as_str()).collect();
        assert_eq!(ids.len(), u.len(), "instrument_id collision across regions");
        let isins: HashSet<&str> = u.iter().map(|s| s.isin.as_str()).collect();
        assert_eq!(isins.len(), u.len(), "ISIN collision across regions");
    }

    #[test]
    fn every_universe_isin_is_iso6166_check_valid() {
        for s in government_universe() {
            assert!(
                is_well_formed(&s.isin),
                "ISIN {} for {} failed the ISO-6166 check",
                s.isin,
                s.instrument_id
            );
        }
    }

    // --- oracle: every coupon-bearing schedule round-trips on the real bond leaf ---

    /// Map a `celnet-refdata` civil date onto a `time::Date`, or `None` if the triple
    /// is not a real calendar date (never for a validated spec).
    fn to_date(d: CivilYmd) -> Option<time::Date> {
        let month = u8::try_from(d.month)
            .ok()
            .and_then(|m| time::Month::try_from(m).ok())?;
        let day = u8::try_from(d.day).ok()?;
        time::Date::from_calendar_date(d.year, month, day).ok()
    }

    /// Map a spec frequency label onto the engine payment frequency.
    fn frequency(label: &str) -> Option<celnet_bond::PaymentFrequency> {
        use celnet_bond::PaymentFrequency::{Annual, Quarterly, SemiAnnual};
        match label {
            "annual" => Some(Annual),
            "semi_annual" => Some(SemiAnnual),
            "quarterly" => Some(Quarterly),
            _ => None,
        }
    }

    #[test]
    fn coupon_bearing_specs_round_trip_price_yield_on_the_real_leaf() {
        use celnet_bond::{AccrualBasis, Bond, dirty_price, yield_to_maturity};

        // A fixed valuation date consistent with the LP-SIM feed; the snapshot's
        // securities all mature after it. Deterministic (no wall clock).
        let settle = time::Date::from_calendar_date(2026, time::Month::April, 16)
            .expect("valuation date is real");

        let mut checked = 0usize;
        for s in government_universe() {
            if s.coupon_type == "zero" {
                continue; // Bills / zeros: no coupon schedule to round-trip here.
            }
            let (Some(maturity), Some(freq)) =
                (to_date(s.maturity_date), frequency(s.coupon_frequency))
            else {
                panic!(
                    "coupon-bearing spec {} has a bad date/frequency",
                    s.instrument_id
                );
            };
            if maturity <= settle {
                continue; // an already-matured snapshot line — nothing to price.
            }
            // act/act is not an engine basis; use 30/360 (the same closest-basis choice
            // the LP-SIM feed makes) — the round-trip below inverts and re-prices under
            // the SAME basis, so it validates the leaf's self-consistency on the real
            // schedule regardless of the display convention.
            let bond = Bond::new(
                settle,
                maturity,
                s.coupon_rate,
                freq,
                AccrualBasis::Thirty360BondBasis,
                s.redemption,
            )
            .unwrap_or_else(|e| panic!("real schedule rejected for {}: {e:?}", s.instrument_id));

            // Invert a par dirty price to a yield on the real leaf, then re-price that
            // yield: the round-trip must close to within solver tolerance.
            let y = yield_to_maturity(&bond, 100.0)
                .unwrap_or_else(|e| panic!("ytm failed for {}: {e:?}", s.instrument_id));
            let reprice = dirty_price(&bond, y)
                .unwrap_or_else(|e| panic!("reprice failed for {}: {e:?}", s.instrument_id));
            assert!(
                (reprice - 100.0).abs() < 1e-6,
                "price↔yield round-trip did not close for {}: repriced {reprice}",
                s.instrument_id
            );
            checked += 1;
        }
        assert!(
            checked >= 30,
            "expected many coupon-bearing bonds, checked {checked}"
        );
    }
}
