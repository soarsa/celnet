//! Per-risk-book risk aggregation, rolled up the book tree
//! (`docs/FI-RISK-ROUTING-REQUIREMENTS.md` §5, §8.5).
//!
//! A [`PositionStore`] buckets each routed fill under the `risk_book_id` the
//! firm-wide graph resolved for it ([`PositionStore::positions_in_risk_book`]).
//! This module reduces those per-book fact sets into the metrics the per-book
//! risk dashboard needs, and — because risk books form a tree ([`RiskBookDef`]'s
//! `parent_id`) — **rolls a parent up over its whole subtree**: a parent's risk is
//! its own routed positions PLUS every descendant's. The union is over
//! *distinct* book ids (a book, and each of its descendants, are distinct ids and
//! a fact is routed to exactly one), so no position is double-counted.
//!
//! # What is REAL here vs deferred
//!
//! Every metric this module emits is computed from the facts actually present in
//! the store — nothing is fabricated (guardrail #2):
//!
//! - **net / gross notional, position count** — summed from each fact's signed
//!   base notional ([`PositionRisk::notional_base`]).
//! - **delta / gamma / vega / theta** — summed from each fact's **canonical leaf**
//!   Greeks ([`CanonicalGreeks`]), which are the *additive* leaf measures (§2.5):
//!   they are already scaled by the position's notional and carry a single
//!   convention, so the cube (and this roll-up) sums them directly. The summed
//!   units are therefore the canonical-leaf units: delta is a **base-leg hedge
//!   amount** (units of the base/asset leg), gamma is `∂²V/∂S² × notional`, vega is
//!   `∂V/∂σ` per `1.0` absolute vol in **premium currency × notional**, theta is
//!   `∂V/∂t` per year `× notional`. (Vega across mixed premium currencies is summed
//!   nominally here; a currency-correct vega netting is a presentation-layer concern
//!   tracked on the cube, §2.3 — flagged, not silently wrong, because the FI books
//!   this view serves are single-numeraire in practice.)
//! - **limit utilization** — for each cap present on the book's [`RiskLimits`] that
//!   is computable at this seam (net / gross notional, and **DV01 whenever the subtree
//!   holds a rates position**), `used / limit` as a fraction plus a green/amber/red
//!   band. A cap whose numerator is genuinely unavailable — a DV01 cap on an FX-only
//!   book — still publishes NO row, because a fabricated `0 / cap` would read as full
//!   headroom on a book whose exposure is merely unknown here.
//!
//! - **net / gross notional, position count** ALSO include the **linear-rates**
//!   positions routed into the book (the [`RatesPositionStore`] seam): a rates fill
//!   contributes its trade-direction notional and increments the count exactly as an FX
//!   leg does. Rates notional is summed nominally into the same net/gross figure as the
//!   FX base-leg notional — single-numeraire in practice, consistent with the vega caveat.
//! - **DV01** — present iff the book's subtree holds a rates position: the summed signed
//!   **linear PV01 proxy** (`rates_linear_exposure`, `notional · tenor · 1bp`), a
//!   conservative undiscounted DV01. A FX-only book carries no rates DV01, so its `dv01`
//!   stays ABSENT (never a fabricated zero) — backward-compatible. The exact curve-
//!   bootstrapped key-rate DV01 ladder is the dedicated rates-risk seam (§5.3).
//!
//! One metric family is **genuinely not evaluable** at this seam and is carried as
//! ABSENT ([`Option::None`]) rather than a fabricated zero:
//!
//! - **live mark-to-market PnL** — needs a mark pass that does not exist at this
//!   booking-seam view (§5.4).

use celnet_proto::RatesPosition;
use celnet_risk_cube::RiskFact;

use crate::config::identity::{IdentityStore, RiskBookDef, RiskLimits};
use crate::services::rates_book::{
    RatesPositionStore, rates_linear_exposure, rates_signed_notional,
};

use super::store::PositionStore;

/// The traffic-light band for a limit utilization (`docs/FI-RISK-ROUTING-REQUIREMENTS.md`
/// §5): GREEN below 0.8, AMBER in `[0.8, 1.0)`, RED at or above 1.0 (a breach).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RagBand {
    /// `used / limit < 0.8` — comfortable headroom.
    Green,
    /// `used / limit` in `[0.8, 1.0)` — approaching the cap.
    Amber,
    /// `used / limit >= 1.0` — at or over the cap.
    Red,
}

/// The AMBER threshold: utilization at or above this (but below [`RAG_RED`]) is amber.
const RAG_AMBER: f64 = 0.8;
/// The RED threshold: utilization at or above this is a breach.
const RAG_RED: f64 = 1.0;

impl RagBand {
    /// The band for a utilization fraction. `NaN` never reaches here (the fraction is
    /// computed defensively, never `0/0`); a `+inf` fraction (used > 0 against a zero
    /// cap) is `>= RAG_RED` and so correctly RED.
    #[must_use]
    fn from_fraction(fraction: f64) -> Self {
        if fraction >= RAG_RED {
            Self::Red
        } else if fraction >= RAG_AMBER {
            Self::Amber
        } else {
            Self::Green
        }
    }
}

/// One book's utilization of a single limit cap (§5).
#[derive(Debug, Clone, PartialEq)]
pub struct LimitUtilization {
    /// The metric this cap governs (`net_notional` / `gross_notional`).
    pub metric: &'static str,
    /// The book's used magnitude for this metric (net uses the absolute of the signed sum).
    pub used: f64,
    /// The configured cap from the book's [`RiskLimits`].
    pub limit: f64,
    /// `used / limit` (0 when the cap is 0 and nothing is used; `+inf` when the cap is
    /// 0 and something is used — a breach).
    pub fraction: f64,
    /// The traffic-light band derived from [`fraction`](Self::fraction).
    pub band: RagBand,
}

impl LimitUtilization {
    /// Build a utilization row from a `used` magnitude and a cap. The fraction is
    /// computed defensively so a zero cap never yields `NaN`: `0/0` is 0 (nothing used,
    /// no capacity, still green), `x/0` for `x > 0` is `+inf` (a breach → red).
    #[must_use]
    fn new(metric: &'static str, used: f64, limit: f64) -> Self {
        let fraction = if limit > 0.0 {
            used / limit
        } else if used > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };
        Self {
            metric,
            used,
            limit,
            fraction,
            band: RagBand::from_fraction(fraction),
        }
    }
}

/// One risk book's aggregated risk, rolled up its subtree (§5).
#[derive(Debug, Clone, PartialEq)]
pub struct RiskBookRisk {
    /// The risk book this row is for ([`RiskBookDef::id`]).
    pub book_id: String,
    /// The book's human-friendly name ([`RiskBookDef::name`]).
    pub name: String,
    /// Net (signed sum) base-currency notional across the subtree's positions.
    pub net_notional: f64,
    /// Gross (sum of absolute) base-currency notional across the subtree.
    pub gross_notional: f64,
    /// The number of positions rolled into this book (own + descendants').
    pub position_count: u32,
    /// Canonical, premium-excluded delta (base-leg hedge amount) × notional, summed.
    pub delta: f64,
    /// Canonical gamma (`∂²V/∂S²`) × notional, summed.
    pub gamma: f64,
    /// Canonical vega (`∂V/∂σ` per 1.0 vol, premium-ccy) × notional, summed.
    pub vega: f64,
    /// Canonical theta (`∂V/∂t` per year) × notional, summed.
    pub theta: f64,
    /// Net DV01 (signed linear PV01 proxy) — present iff the subtree holds a rates
    /// position (summed `rates_linear_exposure`); ABSENT for a FX-only book (§5.3).
    pub dv01: Option<f64>,
    /// Live mark-to-market PnL — ABSENT: no mark pass exists at this seam (§5.4).
    pub pnl: Option<f64>,
    /// Per-cap limit utilization for the caps present on the book AND computable now.
    pub limits: Vec<LimitUtilization>,
}

/// Reduce a book's rolled-up fact set into its [`RiskBookRisk`], computing limit
/// utilization against the book's own [`RiskLimits`]. Pure — the `facts` are already
/// the union of the book's own + descendants' positions (see [`aggregate_risk_book`]);
/// this is the numeric core, split out so it is testable without a store.
///
/// Notional is summed in the position's **base-leg currency** ([`PositionRisk::notional_base`]);
/// Greeks are summed in canonical-leaf units (see the module docs). DV01 and PnL are
/// not evaluable at this seam and are left `None`.
#[must_use]
fn aggregate_facts(
    book: &RiskBookDef,
    facts: &[RiskFact],
    rates: &[RatesPosition],
) -> RiskBookRisk {
    let mut net_notional = 0.0;
    let mut gross_notional = 0.0;
    let mut delta = 0.0;
    let mut gamma = 0.0;
    let mut vega = 0.0;
    let mut theta = 0.0;
    for f in facts {
        let n = f.measure.position.notional_base;
        net_notional += n;
        gross_notional += n.abs();
        let g = &f.measure.leaf.greeks;
        delta += g.delta_base;
        gamma += g.gamma;
        vega += g.vega;
        theta += g.theta;
    }
    // Linear-rates positions routed into this book contribute their trade-direction notional
    // (net/gross) and a signed **DV01** (the existing undiscounted linear PV01 proxy,
    // `rates_linear_exposure`). Rates carry no option Greeks, so delta/gamma/vega/theta stay
    // FX-vanilla. Rates notional is summed nominally into the same net/gross figure as the FX
    // base-leg notional — single-numeraire in practice for the FI books this view serves,
    // consistent with the module's mixed-premium-ccy vega caveat.
    let mut dv01_sum = 0.0;
    for p in rates {
        let signed = rates_signed_notional(p);
        net_notional += signed;
        gross_notional += signed.abs();
        dv01_sum += rates_linear_exposure(p);
    }
    // DV01 is present iff this book (or its subtree) actually holds a rates position — a
    // FX-only book still reports DV01 ABSENT (never a fabricated zero), backward-compatible.
    let dv01 = if rates.is_empty() {
        None
    } else {
        Some(dv01_sum)
    };

    // `usize -> u32`: a live risk book is far under 4 billion routed lines; saturate
    // rather than wrap on the theoretical overflow (never a silent truncation).
    let position_count = u32::try_from(facts.len().saturating_add(rates.len())).unwrap_or(u32::MAX);

    let limits = book
        .limits
        .as_ref()
        .map(|l| limit_utilizations(l, net_notional, gross_notional, dv01))
        .unwrap_or_default();

    RiskBookRisk {
        book_id: book.id.clone(),
        name: book.name.clone(),
        net_notional,
        gross_notional,
        position_count,
        delta,
        gamma,
        vega,
        theta,
        // Present when the subtree holds rates positions (their summed linear PV01 proxy),
        // else ABSENT for a FX-only book — never a fabricated zero.
        dv01,
        pnl: None,
        limits,
    }
}

/// The utilization rows for the caps present on `limits` that are computable at this
/// seam. `max_net_notional` uses the **absolute** of the signed net (a short book still
/// consumes net capacity); `max_gross_notional` uses the gross.
///
/// `max_dv01` publishes a row **iff a DV01 numerator exists** — i.e. the subtree actually
/// holds a rates position. The original rule stands and is the reason for the `Option`:
/// a utilization with no numerator would be a fabricated number, so an FX-only book with
/// a DV01 cap configured still reports NO dv01 row rather than an invented `0 / cap`
/// (which would read as "plenty of headroom" on a book whose DV01 is simply unknown
/// here). What changed is only that the numerator now reaches this function: it was
/// already computed a few lines above in [`aggregate_facts`] and merely never passed in.
///
/// Like the net row, the DV01 magnitude is the **absolute** of the signed sum — a
/// received-fixed book consumes DV01 capacity exactly as a paid-fixed one does.
fn limit_utilizations(
    limits: &RiskLimits,
    net_notional: f64,
    gross_notional: f64,
    dv01: Option<f64>,
) -> Vec<LimitUtilization> {
    let mut out = Vec::new();
    if let Some(cap) = limits.max_net_notional {
        out.push(LimitUtilization::new(
            "net_notional",
            net_notional.abs(),
            cap,
        ));
    }
    if let Some(cap) = limits.max_gross_notional {
        out.push(LimitUtilization::new("gross_notional", gross_notional, cap));
    }
    if let (Some(cap), Some(used)) = (limits.max_dv01, dv01) {
        out.push(LimitUtilization::new("dv01", used.abs(), cap));
    }
    out
}

/// Aggregate a risk book's risk rolled up its subtree (§5): union the book's own routed
/// positions with every descendant's, then reduce via [`aggregate_facts`]. Pure and
/// cycle-safe (the descendant walk is [`IdentityStore::risk_book_descendants`]).
///
/// The union is over distinct book ids, and the store routes each fact to exactly one
/// book id, so a position is counted once.
#[must_use]
pub fn aggregate_risk_book(
    store: &PositionStore,
    rates: Option<&RatesPositionStore>,
    identity: &IdentityStore,
    book: &RiskBookDef,
) -> RiskBookRisk {
    let mut facts = store.positions_in_risk_book(&book.id);
    let mut rates_positions: Vec<RatesPosition> = rates
        .map(|r| r.positions_in_risk_book(&book.id))
        .unwrap_or_default();
    for descendant in identity.risk_book_descendants(&book.id) {
        facts.extend(store.positions_in_risk_book(&descendant.id));
        if let Some(r) = rates {
            rates_positions.extend(r.positions_in_risk_book(&descendant.id));
        }
    }
    aggregate_facts(book, &facts, &rates_positions)
}

/// Aggregate **every enabled risk book** over the live position store, each rolled up its
/// subtree — the push-stream analogue of `AuthEdge::list_risk_book_risk`. It reuses the
/// exact phase-5 [`aggregate_risk_book`] helper the polled RPC uses, over the store's own
/// reconciled [`risk-book tree`](PositionStore::risk_book_tree) (primed at boot and on every
/// admin write beside the routing graph), so the streamed roster is identical to the polled
/// one — no separate identity handle is threaded onto the streaming session.
///
/// The subtree walk needs an [`IdentityStore`] view of the book tree; the store carries the
/// full [`RiskBookDef`] set, so a **transient** identity is built from it (only `risk_books`
/// populated — the walk reads nothing else). Books are returned in registry order and only
/// **enabled** books are rolled up (their disabled descendants still contribute, exactly as
/// the RPC's `aggregate_risk_book` includes all descendants) — matching the RPC's
/// `.filter(|b| b.enabled)` roster.
#[must_use]
pub fn aggregate_enabled_risk_books(
    store: &PositionStore,
    rates: Option<&RatesPositionStore>,
) -> Vec<RiskBookRisk> {
    let identity = IdentityStore {
        risk_books: store.risk_book_tree(),
        ..IdentityStore::default()
    };
    identity
        .risk_books
        .iter()
        .filter(|b| b.enabled)
        .map(|b| aggregate_risk_book(store, rates, &identity, b))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_risk_cube::{
        BookId as CubeBookId, DeskId, EntityId, FactKey, FactMeasure, LocationId, PositionId,
        TraderId,
    };
    use celnet_risk_normalize::{CanonicalGreeks, CanonicalLeaf, PositionRisk};
    use celnet_types::{
        Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, Underlying, VanillaInputs,
    };

    use crate::config::identity::{IdentityStore, RiskBookEdit, RiskLimits};
    use crate::services::risk::store::PositionStore;

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    /// A hand-built vanilla fact with an explicit notional and a delta/vega set proportional
    /// to it — enough to exercise the sums and RAG bands without invoking a pricer.
    fn hand_fact(id: u32, notional: f64) -> RiskFact {
        let position = PositionRisk::fx(
            eurusd(),
            OptionType::Call,
            notional,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        );
        let greeks = CanonicalGreeks {
            delta_base: notional * 0.5,
            gamma: notional * 1e-6,
            vega: notional * 2e-3,
            theta: notional * -1e-4,
            vanna: 0.0,
            volga: 0.0,
            charm: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
        };
        let leaf = CanonicalLeaf {
            underlying: Underlying::Fx(eurusd()),
            spot: 1.10,
            greeks,
            premium_quote: 0.0,
            vega_premium_ccy: Ccy::USD,
            quoted_was_premium_adjusted: false,
        };
        RiskFact {
            position_id: PositionId(id),
            key: FactKey {
                trader: TraderId(1),
                book: CubeBookId(1),
                desk: DeskId(1),
                underlying: Underlying::Fx(eurusd()),
                location: LocationId(1),
                entity: EntityId(1),
            },
            measure: FactMeasure {
                leaf,
                position,
                exotic: None,
            },
            surface_version: 1,
        }
    }

    fn book_with_limits(id: &str, limits: Option<RiskLimits>) -> RiskBookDef {
        RiskBookDef {
            id: id.to_owned(),
            name: id.to_uppercase(),
            parent_id: None,
            desk_id: None,
            description: String::new(),
            limits,
            asset_class: crate::config::identity::default_risk_book_asset_class(),
            enabled: true,
        }
    }

    /// The additive sums are the plain sum of the facts' notionals and canonical Greeks;
    /// net is signed, gross is absolute, and DV01/PnL stay ABSENT (never a fabricated 0).
    #[test]
    fn aggregate_facts_sums_notional_and_greeks() {
        let book = book_with_limits("b", None);
        // +100 long and -40 short: net 60, gross 140.
        let facts = [hand_fact(1, 100.0), hand_fact(2, -40.0)];
        let agg = aggregate_facts(&book, &facts, &[]);
        assert_eq!(agg.position_count, 2);
        assert!((agg.net_notional - 60.0).abs() < 1e-9);
        assert!((agg.gross_notional - 140.0).abs() < 1e-9);
        // delta_base = 0.5 * notional summed: 50 + (-20) = 30.
        assert!((agg.delta - 30.0).abs() < 1e-9);
        // vega = 2e-3 * notional summed: 0.2 + (-0.08) = 0.12.
        assert!((agg.vega - 0.12).abs() < 1e-12);
        // Genuinely-unavailable metrics are absent, not zeroed.
        assert_eq!(agg.dv01, None);
        assert_eq!(agg.pnl, None);
        // No limits configured ⇒ no utilization rows.
        assert!(agg.limits.is_empty());
    }

    /// A 10mm 5y OIS: DV01 = 10,000,000 × 5 × 1bp = 5,000. Against a 10,000 cap that is
    /// half the capacity, so the row publishes 0.5 / GREEN. The numerator was ALWAYS
    /// computed here — it simply never reached `limit_utilizations`.
    #[test]
    fn dv01_utilization_publishes_when_the_subtree_holds_rates() {
        use celnet_proto::{OisInstrument, RatesInstrument, Side, rates_instrument};

        let book = book_with_limits(
            "b",
            Some(RiskLimits {
                max_net_notional: None,
                max_gross_notional: None,
                max_dv01: Some(10_000.0),
            }),
        );
        let ois = RatesPosition {
            position_id: 0,
            entity: 1,
            book: 10,
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years: 5,
                    fixed_rate: 0.04,
                    notional: 10_000_000.0,
                    side: Side::Buy as i32,
                })),
            }),
            ..Default::default()
        };
        let agg = aggregate_facts(&book, &[], &[ois]);

        assert_eq!(agg.dv01, Some(5_000.0), "10mm × 5y × 1bp");
        let row = agg
            .limits
            .iter()
            .find(|u| u.metric == "dv01")
            .expect("a rates-holding book with a DV01 cap publishes the row");
        assert!((row.used - 5_000.0).abs() < 1e-9);
        assert!((row.limit - 10_000.0).abs() < 1e-9);
        assert!((row.fraction - 0.5).abs() < 1e-12);
        assert_eq!(row.band, RagBand::Green);
    }

    /// The received-fixed (short) side consumes DV01 capacity exactly as the paid-fixed
    /// side does: the row uses the ABSOLUTE of the signed sum, mirroring the net-notional
    /// row. A book cannot buy headroom by flipping direction.
    #[test]
    fn dv01_utilization_uses_the_absolute_of_the_signed_sum() {
        use celnet_proto::{OisInstrument, RatesInstrument, Side, rates_instrument};

        let book = book_with_limits(
            "b",
            Some(RiskLimits {
                max_net_notional: None,
                max_gross_notional: None,
                max_dv01: Some(10_000.0),
            }),
        );
        let sell = RatesPosition {
            position_id: 0,
            entity: 1,
            book: 10,
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years: 5,
                    fixed_rate: 0.04,
                    notional: 10_000_000.0,
                    side: Side::Sell as i32,
                })),
            }),
            ..Default::default()
        };
        let agg = aggregate_facts(&book, &[], &[sell]);

        // Signed DV01 is negative (receive-fixed is the opposite IR sign)…
        assert_eq!(agg.dv01, Some(-5_000.0));
        let row = agg.limits.iter().find(|u| u.metric == "dv01").expect("row");
        // …but the utilization is the magnitude, so it consumes the same 50% of the cap.
        assert!((row.used - 5_000.0).abs() < 1e-9);
        assert!((row.fraction - 0.5).abs() < 1e-12);
    }

    /// The original rule is preserved: an FX-ONLY book with a DV01 cap configured
    /// publishes NO dv01 row. A fabricated `0 / cap` would read as full headroom on a
    /// book whose DV01 is merely unknown at this seam, which is worse than silence.
    #[test]
    fn dv01_utilization_is_absent_without_a_numerator() {
        let book = book_with_limits(
            "b",
            Some(RiskLimits {
                max_net_notional: Some(1_000.0),
                max_gross_notional: None,
                max_dv01: Some(10_000.0),
            }),
        );
        let agg = aggregate_facts(&book, &[hand_fact(1, 100.0)], &[]);

        assert_eq!(agg.dv01, None, "an FX-only subtree has no DV01");
        assert!(
            agg.limits.iter().all(|u| u.metric != "dv01"),
            "no numerator ⇒ no fabricated dv01 utilization",
        );
        // The notional cap it CAN compute is still published.
        assert!(agg.limits.iter().any(|u| u.metric == "net_notional"));
    }

    /// RAG bands land exactly at the 0.8 / 1.0 boundaries: 0.8 ⇒ AMBER, 1.0 ⇒ RED,
    /// below 0.8 ⇒ GREEN. Net uses |signed sum|; gross uses the absolute sum.
    #[test]
    fn limit_utilization_rag_bands_at_boundaries() {
        let limits = RiskLimits {
            max_net_notional: Some(100.0),
            max_gross_notional: Some(100.0),
            max_dv01: Some(5.0),
        };
        let book = book_with_limits("b", Some(limits));

        // net = gross = 79 ⇒ 0.79 < 0.8 ⇒ GREEN.
        let green = aggregate_facts(&book, &[hand_fact(1, 79.0)], &[]);
        assert_eq!(green.limits.len(), 2, "net + gross; dv01 skipped (no DV01)");
        assert!(green.limits.iter().all(|u| u.band == RagBand::Green));

        // net = gross = 80 ⇒ exactly 0.8 ⇒ AMBER (the lower boundary is inclusive).
        let amber = aggregate_facts(&book, &[hand_fact(1, 80.0)], &[]);
        for u in &amber.limits {
            assert!((u.fraction - 0.8).abs() < 1e-12);
            assert_eq!(u.band, RagBand::Amber);
        }

        // net = gross = 100 ⇒ exactly 1.0 ⇒ RED (the cap is a breach at equality).
        let red = aggregate_facts(&book, &[hand_fact(1, 100.0)], &[]);
        for u in &red.limits {
            assert!((u.fraction - 1.0).abs() < 1e-12);
            assert_eq!(u.band, RagBand::Red);
        }

        // A short book still consumes NET capacity via the absolute of the signed sum.
        let short = aggregate_facts(&book, &[hand_fact(1, -100.0)], &[]);
        let net = short
            .limits
            .iter()
            .find(|u| u.metric == "net_notional")
            .expect("net cap present");
        assert!((net.used - 100.0).abs() < 1e-9);
        assert_eq!(net.band, RagBand::Red);
        // The dv01 cap is present on the book but NOT emitted (DV01 is absent).
        assert!(short.limits.iter().all(|u| u.metric != "max_dv01"));
    }

    /// A firm-wide graph: notional > 50 → PARENT, else → CHILD (a 2-leaf split).
    fn split_graph() -> celnet_risk_routing::RiskRoutingGraph {
        use celnet_risk_routing::{RouteField, RouteOp, RouteValue, RoutingNode};
        use std::collections::BTreeMap;
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0u32,
            RoutingNode::Condition {
                field: RouteField::Notional,
                op: RouteOp::Gt,
                value: RouteValue::Num(50.0),
                on_true: 1,
                on_false: 2,
            },
        );
        nodes.insert(
            1u32,
            RoutingNode::Book {
                risk_book_id: "parent".to_owned(),
            },
        );
        nodes.insert(
            2u32,
            RoutingNode::Book {
                risk_book_id: "child".to_owned(),
            },
        );
        celnet_risk_routing::RiskRoutingGraph { entry: 0, nodes }
    }

    fn booked(id: u64, notional: f64) -> crate::services::risk::store::BookedPosition {
        crate::services::risk::store::BookedPosition {
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

    /// A parent book aggregates its OWN routed positions PLUS its child's — the tree
    /// roll-up. Ground truth is the store's own per-book fact sets, so the roll-up must
    /// be exactly their union (count + notional + summed delta).
    #[test]
    fn aggregate_risk_book_rolls_up_the_subtree() {
        // Identity: CHILD nested under PARENT (a 2-level tree).
        let mut identity = IdentityStore::default();
        let parent = identity
            .create_risk_book(RiskBookEdit {
                name: "Parent".to_owned(),
                parent_id: None,
                desk_id: None,
                description: String::new(),
                limits: None,
                asset_class: crate::config::identity::default_risk_book_asset_class(),
                enabled: true,
            })
            .expect("create parent");
        let _child = identity
            .create_risk_book(RiskBookEdit {
                name: "Child".to_owned(),
                parent_id: Some(parent.id.clone()),
                desk_id: None,
                description: String::new(),
                limits: None,
                asset_class: crate::config::identity::default_risk_book_asset_class(),
                enabled: true,
            })
            .expect("create child");
        assert_eq!(parent.id, "parent");

        // Store: route one fill (60 > 50) to PARENT, one (10) to CHILD.
        let store = PositionStore::new();
        store.set_routing(Some(split_graph()));
        store
            .book_from_attribution(booked(1, 60.0), &attribution("PARENT"))
            .expect("book parent fill");
        store
            .book_from_attribution(booked(2, 10.0), &attribution("CHILD"))
            .expect("book child fill");

        // Ground truth: the raw per-book fact sets (the roll-up must be their union).
        let own: Vec<RiskFact> = store.positions_in_risk_book("parent");
        let child_facts: Vec<RiskFact> = store.positions_in_risk_book("child");
        assert_eq!(own.len(), 1, "one fill routed to parent");
        assert_eq!(child_facts.len(), 1, "one fill routed to child");
        let expect_delta: f64 = own
            .iter()
            .chain(child_facts.iter())
            .map(|f| f.measure.leaf.greeks.delta_base)
            .sum();

        // The child alone rolls up ONLY its own fill.
        let child_def = identity.risk_book("child").expect("child def");
        let child_agg = aggregate_risk_book(&store, None, &identity, child_def);
        assert_eq!(child_agg.position_count, 1);
        assert!((child_agg.net_notional - 10.0).abs() < 1e-9);

        // The parent rolls up BOTH: count 2, notional 70, delta = sum of both leaves.
        let parent_agg = aggregate_risk_book(&store, None, &identity, &parent);
        assert_eq!(parent_agg.position_count, 2);
        assert!((parent_agg.net_notional - 70.0).abs() < 1e-9);
        assert!((parent_agg.gross_notional - 70.0).abs() < 1e-9);
        assert!((parent_agg.delta - expect_delta).abs() < 1e-9);
        assert_eq!(parent_agg.dv01, None);
        assert_eq!(parent_agg.pnl, None);
    }

    /// A book aggregates BOTH its FX positions and the linear-rates positions routed into
    /// it: net/gross notional + count sum both stores, and the rates DV01 (linear PV01 proxy)
    /// is surfaced — while a FX-only aggregation keeps DV01 absent (backward-compatible).
    #[test]
    fn aggregate_includes_routed_rates_positions() {
        use crate::services::rates_book::RatesPositionStore;
        use celnet_proto::{OisInstrument, RatesInstrument, RatesPosition, Side, rates_instrument};
        use celnet_risk_routing::{RiskRoutingGraph, RoutingNode};
        use std::collections::BTreeMap;

        // A single-book graph: every fill → risk book "b" (routing is book-resolved, not
        // attribution-resolved, so the same graph serves the FX and the rates fill).
        let single = || {
            let mut nodes = BTreeMap::new();
            nodes.insert(
                0u32,
                RoutingNode::Book {
                    risk_book_id: "b".to_owned(),
                },
            );
            RiskRoutingGraph { entry: 0, nodes }
        };

        // FX store: one +100 fill routed into "b".
        let fx = PositionStore::new();
        fx.set_routing(Some(single()));
        fx.book_from_attribution(booked(1, 100.0), &attribution("HELD"))
            .expect("fx book");

        // Rates store: one 10mm 5y pay-fixed OIS routed into "b".
        let rates = RatesPositionStore::new();
        rates.set_routing(Some(single()));
        let ois = RatesPosition {
            position_id: 0,
            entity: 1,
            book: 10,
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years: 5,
                    fixed_rate: 0.04,
                    notional: 10_000_000.0,
                    side: Side::Buy as i32,
                })),
            }),
            ..Default::default()
        };
        let booked_rates = rates.book(ois).expect("rates book");
        assert_eq!(
            rates.risk_book_of(booked_rates.position_id).as_deref(),
            Some("b"),
            "the rates fill routes into the resolved risk book",
        );

        let book = book_with_limits("b", None);
        let identity = IdentityStore::default();
        let agg = aggregate_risk_book(&fx, Some(&rates), &identity, &book);

        // Count = 1 FX + 1 rates.
        assert_eq!(agg.position_count, 2);
        // Net/gross notional = FX 100 + rates trade-direction notional (+10mm pay-fixed).
        assert!((agg.net_notional - (100.0 + 10_000_000.0)).abs() < 1e-3);
        assert!((agg.gross_notional - (100.0 + 10_000_000.0)).abs() < 1e-3);
        // DV01 now present: the rates linear PV01 proxy 10mm · 5 · 1bp = 5000.
        let dv01 = agg.dv01.expect("rates positions surface a DV01");
        assert!((dv01 - 5000.0).abs() < 1e-6);

        // FX-only aggregation (no rates store) keeps DV01 ABSENT — backward-compatible.
        let fx_only = aggregate_risk_book(&fx, None, &identity, &book);
        assert_eq!(fx_only.dv01, None);
        assert_eq!(fx_only.position_count, 1);
    }

    fn attribution(book: &str) -> celnet_proto::AttributionRecord {
        use celnet_proto::{BookId, Owner, owner};
        celnet_proto::AttributionRecord {
            quoted_by: Some(BookId {
                book: "AUTO-MM".to_owned(),
                owner: Some(Owner {
                    seat: Some(owner::Seat::AutoPricer("celnet-auto-pricer".to_owned())),
                }),
            }),
            held_by: Some(BookId {
                book: book.to_owned(),
                owner: Some(Owner {
                    seat: Some(owner::Seat::Trader("jdoe".to_owned())),
                }),
            }),
            won: Some(true),
            lp_count: Some(3),
        }
    }
}
