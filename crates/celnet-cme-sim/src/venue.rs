//! The **listed venue** itself: how the exchange resolves what a taker named, how
//! it sizes a quote in whole contracts, and how it stands up its quoting and trading
//! halves.
//!
//! # Why a product symbol must resolve, not just a contract code
//!
//! A hedge vehicle is routinely configured as a **product** (`ZF` — "the 5-Year
//! note future") rather than a specific delivery month (`ZFU26`), because the desk
//! wants "the front contract", whatever that is this quarter. Only the venue can
//! answer that: it is the one component that knows which cycles are still trading at
//! the valuation date. [`resolve_contract`] therefore accepts either form and rolls
//! a product symbol to its front delivery month through
//! [`celnet_refdata::front_contract_id`].
//!
//! An **explicit** delivery month is never re-pointed. If a taker names `ZFU26`, it
//! gets `ZFU26` or it gets an honest miss — silently substituting a different
//! contract for the one a taker named would be the single worst thing a venue could
//! do, since the two settle on different dates against different deliverables.
//!
//! # Sizes: whole contracts, carried in face
//!
//! A listed contract trades in whole contracts, never in fractions. This venue
//! therefore quotes and fills only whole multiples of the contract's own
//! `contract_face_value` — [`contract_lot_size`] is that lot, and the quoted clip is
//! snapped down to a whole number of them.
//!
//! The size is nonetheless carried on the wire in **face units**, exactly as the
//! cash feed's is, because the quote wire has no size-unit discriminator: an
//! `LpQuote`'s `bid_size` is a face amount everywhere in the estate. A quoted
//! 4,600,000 on a $100,000-face contract is therefore 46 contracts, and that is the
//! reading a DV01-ratio hedge already works in (it converts a cash risk figure to a
//! contract count through the contract's own face value). Changing the unit here
//! would silently rescale every futures hedge in the server by a factor of the
//! contract size.

use celnet_lp_sim::execution::Side;
use celnet_lp_sim::orders::{LiveMarkets, OrderVenue, QuotedMarket};
use celnet_lp_sim::quoted::QuotedLine;
use celnet_lp_sim::roster::{LISTED_ROSTER, SimLpProfile};
use celnet_lp_sim::{LpParams, SimLp, VenueFeed};
use celnet_types::BrokenDate;

use crate::contract::FuturesContract;

/// The venue's stable connection id — the `VenueId` its contributions consolidate
/// under, the `SenderCompID` of its order session, and the `lp_won` a booked futures
/// hedge is attributed to.
pub const VENUE_ID: &str = "cme-sim";

/// The venue's roster profile (the single member of [`LISTED_ROSTER`]).
///
/// # Panics
/// Never in practice: [`LISTED_ROSTER`] is a compile-time constant asserted
/// non-empty by its own tests.
#[must_use]
pub fn profile() -> &'static SimLpProfile {
    celnet_lp_sim::roster::profile_by_id(LISTED_ROSTER, VENUE_ID)
        .expect("the listed roster always carries the cme-sim venue")
}

/// The base half-spread the venue's quoting knobs are expressed relative to.
///
/// Every listed line rescales this to **half its own contract's minimum price
/// increment** through [`QuotedLine::spread_scale`], so the absolute value here only
/// has to match the one the lines were built against. It is the same cash-market
/// base the OTC simulator uses, for exactly that reason.
pub const BASE_HALF_SPREAD: f64 = 2.0e-2;

/// The base per-member skew step the venue's lines are expressed relative to.
///
/// A single-member venue has no panel to lean within, so this only ever scales to
/// zero displacement in practice; it is carried so the listed lines are built with
/// the same two-parameter shape as every other line in the estate.
pub const BASE_SKEW_STEP: f64 = 4.0e-3;

/// The base firm clip, before the venue's own [`SimLpProfile::size_multiple`] and
/// the whole-contract snap. Sized so even the largest contract shows a real market:
/// a $100,000-face contract at 6× this is 60 contracts at the touch.
pub const BASE_SIZE: f64 = 1_000_000.0;

/// How a taker's symbol resolved to a listed contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolResolution {
    /// The taker named a specific delivery month and it is trading.
    ExplicitContract,
    /// The taker named a product symbol, which rolled to its front delivery month.
    RolledToFrontMonth,
}

/// Resolve a taker's `symbol` — either a specific contract code (`ZFU26`) or a
/// product symbol (`ZF`) — against the contracts this venue is currently quoting.
///
/// A product symbol rolls to its front delivery month; an explicit contract code is
/// matched exactly and **never** re-pointed. `None` when the symbol names neither a
/// quoted contract nor a product with a live front month — an honest miss, never a
/// substituted contract.
#[must_use]
pub fn resolve_contract<'a>(
    contracts: &'a [FuturesContract],
    symbol: &str,
    as_of: celnet_refdata::CivilYmd,
) -> Option<(&'a FuturesContract, SymbolResolution)> {
    let wanted = symbol.trim().to_ascii_uppercase();
    if wanted.is_empty() {
        return None;
    }
    // An explicit delivery month wins outright and is matched verbatim.
    if let Some(c) = contracts
        .iter()
        .find(|c| c.instrument_id().eq_ignore_ascii_case(&wanted))
    {
        return Some((c, SymbolResolution::ExplicitContract));
    }
    // A product symbol rolls to whatever cycle is trading at the valuation date.
    if !celnet_refdata::is_product_symbol(&wanted) {
        return None;
    }
    let front = celnet_refdata::front_contract_id(&wanted, as_of)?;
    contracts
        .iter()
        .find(|c| c.instrument_id().eq_ignore_ascii_case(&front))
        .map(|c| (c, SymbolResolution::RolledToFrontMonth))
}

/// The whole-lot size of a listed contract, in the face units the wire carries: one
/// contract's face value.
///
/// `None` for a contract whose face value is not a usable positive number, which the
/// committed universe never contains — a contract with no lot size is not tradeable
/// and this venue declines to quote it rather than inventing one.
#[must_use]
pub fn contract_lot_size(contract: &FuturesContract) -> Option<f64> {
    let face = contract.spec.terms.face_value;
    (face.is_finite() && face > 0.0).then_some(face)
}

/// The firm clip the venue shows on a contract: its base appetite snapped **down**
/// to a whole number of contracts, and never fewer than one.
///
/// Snapped down rather than to the nearest, because rounding up would show size the
/// venue's own appetite does not cover.
#[must_use]
pub fn whole_contract_clip(contract: &FuturesContract, base_size: f64, seed: u64) -> Option<f64> {
    let lot = contract_lot_size(contract)?;
    let appetite = profile().firm_size(base_size, seed);
    let contracts = (appetite / lot).floor().max(1.0);
    Some(contracts * lot)
}

/// Build the venue's single [`SimLp`] over `lines` — the quoting half.
///
/// A listed market has one central book, not a panel of competing dealers, so this
/// is deliberately one member. Its half-spread, refresh cadence, latency and quality
/// come from the venue's own roster profile exactly as an OTC counterparty's do; its
/// per-line `spread_scale` (set by [`FuturesContract::to_line`]) then pins the quote
/// to half the contract's own minimum price increment, and the model's tick grid
/// snaps the bid down and the offer up onto that increment.
#[must_use]
pub fn build_venue_feed(lines: &[QuotedLine], seed: u64) -> SimLp {
    let p = profile();
    let books = lines
        .iter()
        .map(|line| {
            celnet_lp_sim::InstrumentModel::new(line.instrument.clone(), line.mid).with_quote_shape(
                line.spread_scale,
                line.lean_scale,
                line.tick,
                line.dealer_view,
            )
        })
        .collect();
    let params = LpParams {
        half_spread: p.half_spread(BASE_HALF_SPREAD, seed),
        // One central market has nothing to lean against.
        skew: 0.0,
        size: p.firm_size(BASE_SIZE, seed),
        tick_nanos: p.refresh_nanos(),
        latency_nanos: p.latency_nanos(),
        quality: p.quality,
    };
    SimLp::new(
        celnet_aggregation::VenueId::new(VENUE_ID),
        seed,
        params,
        books,
    )
}

/// Build the venue's trading half, sharing `markets` with the quoting half.
#[must_use]
pub fn build_order_venue(markets: LiveMarkets, seed: u64) -> OrderVenue {
    OrderVenue::from_profile(
        profile(),
        profile().half_spread(BASE_HALF_SPREAD, seed),
        markets,
    )
}

/// Publish the venue's current market for every contract in `lines` into `venue`'s
/// tradeable book, from the SAME feed the quotes are streamed from.
///
/// This is the invariant that keeps the venue honest: the level a taker sees in the
/// composite is the level it trades against, because both come from one
/// [`VenueFeed::top_of_book`] call. Sizes are snapped to whole contracts on both
/// sides.
pub fn publish_markets(
    feed: &SimLp,
    order_venue: &OrderVenue,
    lines: &[QuotedLine],
    contracts: &[FuturesContract],
    now_nanos: i64,
) -> usize {
    let mut published = 0;
    for line in lines {
        let Some(q) = feed.top_of_book(&line.instrument, now_nanos) else {
            continue;
        };
        let lot = contracts
            .iter()
            .find(|c| c.instrument_id() == line.instrument_id)
            .and_then(contract_lot_size);
        let snap = |size: f64| match lot {
            Some(l) if l > 0.0 => (size / l).floor().max(1.0) * l,
            _ => size,
        };
        order_venue.publish(
            &line.instrument_id,
            QuotedMarket {
                bid: q.bid,
                offer: q.offer,
                bid_size: snap(q.bid_size),
                offer_size: snap(q.offer_size),
                ts_nanos: q.ts,
                lot_size: lot,
            },
        );
        published += 1;
    }
    published
}

/// The venue's valuation date as a reference-data civil date — the instant the
/// listed universe is filtered and rolled against, so a contract that has stopped
/// trading is never quoted and a product symbol always resolves to a live cycle.
#[must_use]
pub fn as_of(settlement: BrokenDate) -> celnet_refdata::CivilYmd {
    celnet_refdata::CivilYmd::new(
        settlement.year,
        u32::from(settlement.month),
        u32::from(settlement.day),
    )
}

/// The side of the venue's market a taker on `side` executes against — re-exported
/// as a named helper so a caller never has to remember which way round it is.
#[must_use]
pub fn taker_touch(market: &QuotedMarket, side: Side) -> (f64, f64) {
    market.touch(side)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::load_futures_universe;
    use celnet_lp_sim::load_government_universe;

    fn settle() -> BrokenDate {
        BrokenDate::new(2026, 4, 16)
    }

    fn universe() -> Vec<FuturesContract> {
        load_futures_universe(&load_government_universe(false), settle())
    }

    /// A PRODUCT symbol rolls to the front delivery month that is actually trading
    /// at the valuation date — the whole reason a hedge vehicle may name `ZF`.
    #[test]
    fn a_product_symbol_rolls_to_its_front_month() {
        let contracts = universe();
        let (c, how) =
            resolve_contract(&contracts, "ZF", as_of(settle())).expect("ZF must resolve");
        assert_eq!(how, SymbolResolution::RolledToFrontMonth);
        // The resolved code IS the product's front month, not merely "a ZF".
        let front = celnet_refdata::front_contract_id("ZF", as_of(settle())).expect("front month");
        assert_eq!(c.instrument_id(), front);
        assert!(c.instrument_id().starts_with("ZF"));

        // Lower case resolves identically — a taker's casing is not a trading rule.
        let (lc, _) =
            resolve_contract(&contracts, "zf", as_of(settle())).expect("case-insensitive");
        assert_eq!(lc.instrument_id(), c.instrument_id());

        // Every listed product resolves to something live.
        for product in ["ZT", "ZF", "ZN", "TN", "ZB", "UB"] {
            let (c, how) = resolve_contract(&contracts, product, as_of(settle()))
                .unwrap_or_else(|| panic!("{product} did not resolve"));
            assert_eq!(how, SymbolResolution::RolledToFrontMonth);
            assert!(c.instrument_id().starts_with(product));
        }
    }

    /// An EXPLICIT delivery month is matched verbatim and is NEVER re-pointed to the
    /// front month. Substituting a different contract for the one a taker named
    /// would settle on a different date against a different deliverable.
    #[test]
    fn an_explicit_delivery_month_is_never_re_pointed() {
        let contracts = universe();
        let front = celnet_refdata::front_contract_id("ZF", as_of(settle())).expect("front month");
        // Pick a ZF cycle that is NOT the front month, if the universe lists one.
        let back = contracts
            .iter()
            .map(|c| c.instrument_id().to_owned())
            .find(|id| id.starts_with("ZF") && *id != front);

        let (c, how) =
            resolve_contract(&contracts, &front, as_of(settle())).expect("front resolves");
        assert_eq!(how, SymbolResolution::ExplicitContract);
        assert_eq!(c.instrument_id(), front);

        if let Some(back) = back {
            let (c, how) =
                resolve_contract(&contracts, &back, as_of(settle())).expect("back month resolves");
            assert_eq!(how, SymbolResolution::ExplicitContract);
            assert_eq!(
                c.instrument_id(),
                back,
                "a named back month was silently rolled to the front"
            );
        }
    }

    /// An unknown symbol is an honest miss — never the nearest thing the venue has.
    #[test]
    fn an_unknown_symbol_is_a_miss_not_a_substitution() {
        let contracts = universe();
        for bogus in ["", "   ", "XX", "ZFZ99", "912810TZ1"] {
            assert!(
                resolve_contract(&contracts, bogus, as_of(settle())).is_none(),
                "{bogus} should not resolve"
            );
        }
    }

    /// Every quoted clip is a whole number of contracts, on both sides — a listed
    /// contract does not trade in fractions.
    #[test]
    fn every_clip_is_a_whole_number_of_contracts() {
        for c in universe() {
            let lot = contract_lot_size(&c).expect("a listed contract has a face value");
            assert!(lot > 0.0);
            let clip = whole_contract_clip(&c, BASE_SIZE, 0x1234_5678).expect("clip");
            let n = clip / lot;
            assert!(
                (n - n.round()).abs() < 1e-9,
                "{}: clip {clip} is {n} contracts, not a whole number",
                c.instrument_id()
            );
            assert!(
                n >= 1.0,
                "{}: clip is under one contract",
                c.instrument_id()
            );
        }
    }

    /// The venue quotes a tick-aligned two-way on the contract's own published
    /// minimum price increment: the bid snapped DOWN, the offer UP, never crossed,
    /// and one or two ticks wide — which is how these contracts actually trade.
    #[test]
    fn the_quoted_market_is_tick_aligned_and_uncrossed() {
        let contracts = universe();
        let lines: Vec<QuotedLine> = crate::contract::futures_lines(
            &contracts,
            BASE_HALF_SPREAD,
            BASE_SKEW_STEP,
            0.02,
            3.0e-4,
        );
        assert_eq!(
            lines.len(),
            contracts.len(),
            "every contract must be quotable"
        );

        let feed = build_venue_feed(&lines, 0x1234_5678);
        let now = 1_700_000_000_000_000_000;
        for line in &lines {
            let q = feed
                .top_of_book(&line.instrument, now)
                .unwrap_or_else(|| panic!("{} has no market", line.instrument_id));
            let tick = line.tick.expect("a listed line trades on a grid");

            for (label, px) in [("bid", q.bid), ("offer", q.offer)] {
                let ticks = px / tick;
                assert!(
                    (ticks - ticks.round()).abs() < 1e-6,
                    "{}: {label} {px} is not on the {tick} tick grid",
                    line.instrument_id
                );
            }
            assert!(
                q.offer > q.bid,
                "{}: crossed or locked market {} / {}",
                line.instrument_id,
                q.bid,
                q.offer
            );
            let width_ticks = (q.offer - q.bid) / tick;
            assert!(
                (0.5..=3.0).contains(&width_ticks),
                "{}: {width_ticks} ticks wide is not a listed market",
                line.instrument_id
            );
        }
    }

    /// The tradeable book and the published quote are the SAME market: a taker
    /// trades against exactly the level it was shown, in whole contracts.
    #[test]
    fn the_tradeable_book_mirrors_the_published_quote() {
        let contracts = universe();
        let lines = crate::contract::futures_lines(
            &contracts,
            BASE_HALF_SPREAD,
            BASE_SKEW_STEP,
            0.02,
            3.0e-4,
        );
        let seed = 0x1234_5678;
        let feed = build_venue_feed(&lines, seed);
        let markets: LiveMarkets =
            std::sync::Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()));
        let ov = build_order_venue(std::sync::Arc::clone(&markets), seed);
        let now = 1_700_000_000_000_000_000;
        let n = publish_markets(&feed, &ov, &lines, &contracts, now);
        assert_eq!(n, lines.len());

        let book = markets.read().expect("markets");
        for line in &lines {
            let q = feed.top_of_book(&line.instrument, now).expect("market");
            let m = book.get(&line.instrument_id).expect("published");
            assert_eq!(m.bid, q.bid, "{}: bid diverged", line.instrument_id);
            assert_eq!(m.offer, q.offer, "{}: offer diverged", line.instrument_id);
            let lot = m.lot_size.expect("a listed market has a lot size");
            for size in [m.bid_size, m.offer_size] {
                let n = size / lot;
                assert!(
                    (n - n.round()).abs() < 1e-9 && n >= 1.0,
                    "{}: published size {size} is not whole contracts",
                    line.instrument_id
                );
            }
            assert_eq!(taker_touch(m, Side::Buy).0, m.offer);
            assert_eq!(taker_touch(m, Side::Sell).0, m.bid);
        }
    }
}
