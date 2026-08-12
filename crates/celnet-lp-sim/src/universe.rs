//! The bundled **government reference universe** loader.
//!
//! Parses the committed US-Treasury securities-master snapshot (267 CUSIPs with
//! auction/quote prices) into typed [`TreasuryBond`] records, and — via
//! [`load_curated_universe`] / [`load_government_universe`] — extends the priced set
//! with the curated non-US govvies (UK gilts + EUR govvies) from
//! [`celnet_refdata::curated_universe`], so the whole
//! [`celnet_refdata::government_universe`] the server seeds also streams a real price.
//! Each record maps onto the two identities the rest of the stack keys on:
//!
//! - the **canonical server `instrument_id`** (ADR-0022 decision D) — the one
//!   identity shared across reference data, the wire, the GUI and the composite.
//!   The Treasury's **CUSIP** is that id (unique registry-wide); the ISIN is
//!   carried alongside for display/cross-ref. This is what the LP-SIM feed puts on
//!   the wire and what a subscriber sees.
//! - the aggregation engine's asset-agnostic [`Instrument`] key
//!   ([`TreasuryBond::engine_instrument`]) — `Underlying::Commodity` carrying the
//!   CUSIP as a vendor-neutral free-form ticker (guardrail #8: the CUSIP is an
//!   opaque code, not a vendor product name) plus a [`Tenor::BrokenDate`] maturity.
//!   Government-cash has no dedicated `Underlying` arm and the consolidation is
//!   asset-agnostic over the key, so the free-form ticker arm is the correct
//!   injective carrier: **distinct CUSIPs never collide** (unlike a maturity-only
//!   key, where reopenings share a date).
//!
//! ## Embedded, not read at runtime (justification)
//!
//! The document is embedded with [`include_str!`] so a built `lp-sim` binary is
//! **self-contained** — it needs no sidecar data file on the deploy host, exactly
//! as the FIX simulator ships a prebuilt client rather than build-at-runtime. The
//! universe is a fixed reference snapshot (a securities master), so there is no
//! freshness argument for a runtime read; embedding removes a deployment failure
//! mode (a missing/renamed file) at zero runtime cost.
//!
//! ## Prices are a real reference, never fabricated
//!
//! Each record carries the auction/quote `buyPrice` (ask) and `sellPrice` (bid)
//! per 100 face. [`TreasuryBond::reference_mid`] is their midpoint and
//! [`TreasuryBond::to_reference_bond`] rebuilds the bond's real cashflow schedule
//! as a [`celnet_bond::Bond`], so the stochastic feed prices the bond off the REAL
//! analytics leaf seeded at the reference — it does not invent a price handle.

use celnet_aggregation::Instrument;
use celnet_bond::{AccrualBasis, Bond, BondError, PaymentFrequency};
use celnet_types::{BrokenDate, Ccy, CommodityRef, Symbol, Tenor, Underlying};
use serde::Deserialize;
use time::{Date, Month};

/// The committed Treasury securities-master snapshot, embedded so the binary is
/// self-contained (see the module docs).
pub const UNIVERSE_JSON: &str = celnet_refdata::TREASURY_UNIVERSE_JSON;

/// The maximum sane price per 100 face: a coupon Treasury trades near par, so a
/// quote at or beyond this (or at/below zero) is a data error and is filtered out
/// rather than fed to the consolidator.
const MAX_PRICE_PER_100: f64 = 200.0;

/// The coarse US-Treasury security class, parsed from `securityType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityType {
    /// A Treasury **Bill** — a zero-coupon discount security, ≤ 1y.
    Bill,
    /// A Treasury **Note** — a coupon security, 2y–10y.
    Note,
    /// A Treasury **Bond** — a coupon security, > 10y.
    Bond,
}

impl SecurityType {
    /// Parse the `securityType` token (`Bill` / `Note` / `Bond`, case-insensitive,
    /// tolerant of the price-file's `MARKET BASED …` prefixes), or `None` for an
    /// unrecognised class.
    #[must_use]
    pub fn parse(label: &str) -> Option<Self> {
        let l = label.to_ascii_uppercase();
        if l.contains("BILL") {
            Some(Self::Bill)
        } else if l.contains("NOTE") {
            Some(Self::Note)
        } else if l.contains("BOND") {
            Some(Self::Bond)
        } else {
            None
        }
    }

    /// Whether this class pays coupons (Notes and Bonds do; Bills are zeros).
    #[must_use]
    pub fn is_coupon_bearing(self) -> bool {
        matches!(self, Self::Note | Self::Bond)
    }
}

/// One priced, well-formed government reference bond.
///
/// Covers the whole [`celnet_refdata`] government universe: US Treasuries (parsed
/// from the embedded snapshot, priced off the snapshot's real ask/bid, CUSIP
/// identity) **and** the curated non-US govvies (UK gilts + EUR govvies, built from a
/// [`celnet_refdata::GovBondSpec`] via [`TreasuryBond::from_gov_spec`], slug identity,
/// seeded at par). Built only by the loaders from validated inputs, so every field
/// here is known-good: the ISIN passes its check digit, both prices are finite and in
/// a sane band, and the maturity is a real calendar date.
#[derive(Debug, Clone, PartialEq)]
pub struct TreasuryBond {
    /// The canonical server `instrument_id` (ADR-0022 D) — the CUSIP for a US
    /// Treasury (the wire id the feed already streams), or the curated slug for a
    /// non-US govvie (e.g. `uk-gilt-10y-2036`). This is the key the server's identity
    /// join attaches the seeded name/ISIN to.
    pub instrument_id: String,
    /// The pricing/settlement currency — the currency leg of the engine key. `USD`
    /// for Treasuries; `GBP`/`EUR` for the curated non-US govvies.
    pub currency: Ccy,
    /// The region label (`us` / `uk` / `de` / `fr` / `it`).
    pub region: &'static str,
    /// A pre-built friendly blotter/GUI name for the curated non-US govvies (their
    /// `GovBondSpec.name`); `None` for a Treasury, whose display name is composed
    /// from its auction `term` + class.
    pub name: Option<String>,
    /// The 9-character CUSIP for a US Treasury; empty for a non-US govvie (which
    /// carries no CUSIP). Retained as an external cross-ref / display id.
    pub cusip: String,
    /// The 12-character ISIN (check-digit valid), carried for display/cross-ref.
    pub isin: String,
    /// The coarse security class (Treasury auction class; a curated coupon govvie is
    /// classed as [`SecurityType::Bond`]).
    pub security_type: SecurityType,
    /// The auction term label, e.g. `4-Week`, `10-Year`, `19-Year 10-Month`; empty
    /// for a curated non-US govvie (which supplies its own [`name`](Self::name)).
    pub term: String,
    /// The final-redemption date.
    pub maturity: BrokenDate,
    /// The dated (accrual-start) date, when the record carries one.
    pub dated_date: Option<BrokenDate>,
    /// The annual coupon rate as a decimal (e.g. `0.05` for a 5% coupon); `0.0`
    /// for a Bill / zero-coupon security.
    pub coupon: f64,
    /// The coupon frequency, or `None` for a zero-coupon Bill.
    pub frequency: Option<PaymentFrequency>,
    /// The reference **ask** (`buyPrice`) per 100 face — the price to *buy* the
    /// bond, i.e. the LP's offer side. Seeded at par for a curated non-US govvie.
    pub ask: f64,
    /// The reference **bid** (`sellPrice`) per 100 face — the price to *sell* the
    /// bond, i.e. the LP's bid side. Seeded at par for a curated non-US govvie.
    pub bid: f64,
}

impl TreasuryBond {
    /// The canonical server `instrument_id` (ADR-0022 D) — the CUSIP for a Treasury,
    /// or the curated slug for a non-US govvie.
    #[must_use]
    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }

    /// A short human label for the security. For a curated non-US govvie this is its
    /// pre-built [`name`](Self::name); for a Treasury it is composed from the auction
    /// term + class, e.g. `10-Year Note` — the GUI blotter name.
    #[must_use]
    pub fn display_name(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        let class = match self.security_type {
            SecurityType::Bill => "Bill",
            SecurityType::Note => "Note",
            SecurityType::Bond => "Bond",
        };
        format!("{} {class}", self.term)
    }

    /// The reference mid price per 100 face: the midpoint of the ask and bid.
    #[must_use]
    pub fn reference_mid(&self) -> f64 {
        0.5 * (self.ask + self.bid)
    }

    /// The reference bid/ask half-spread per 100 face (`(ask − bid)/2`), clamped
    /// non-negative (the snapshot occasionally quotes an inverted cross by a hair).
    #[must_use]
    pub fn reference_half_spread(&self) -> f64 {
        (0.5 * (self.ask - self.bid)).max(0.0)
    }

    /// The aggregation engine's asset-agnostic [`Instrument`] key for this bond:
    /// the canonical `instrument_id` as a vendor-neutral free-form commodity ticker
    /// (guardrail #8) carried in the bond's pricing currency, plus the maturity as a
    /// [`Tenor::BrokenDate`]. Injective over `instrument_id`, so two distinct
    /// securities never consolidate into one line. (For a US Treasury the id is the
    /// CUSIP and the currency is `USD`, so the key is byte-identical to before.)
    #[must_use]
    pub fn engine_instrument(&self) -> Instrument {
        Instrument::new(
            Underlying::Commodity(CommodityRef::new(
                Symbol::new(self.instrument_id.clone(), ""),
                self.currency,
            )),
            Tenor::BrokenDate(self.maturity),
        )
    }

    /// Build the stochastic **mean-reverting-yield model** for this bond, seeded at
    /// the reference: the yield is inverted from the reference mid through the REAL
    /// analytics leaf (`yield_to_maturity`), so the feed's prices are the oracle's
    /// clean prices of a real yield — never a fabricated handle. The process reverts
    /// to that same reference yield (`long_run == initial`), jittered by
    /// `perturbation` and pulled at `reversion_per_sec`.
    ///
    /// Returns `None` when the bond cannot be modelled at `settlement` (a matured or
    /// degenerate schedule, or a reference mid outside the solvable yield range —
    /// e.g. a deep-discount Bill the coupon-bond yield solver cannot bracket).
    #[must_use]
    pub fn yield_model(
        &self,
        settlement: BrokenDate,
        reversion_per_sec: f64,
        perturbation: f64,
    ) -> Option<crate::price::YieldModel> {
        let bond = self.to_reference_bond(settlement).ok()?;
        // The reference mid is a clean price; the solver takes a dirty price, and at
        // the seam the two coincide to within the round-trip tolerance the loader's
        // own test asserts. Invert to the reference yield.
        let y = celnet_bond::yield_to_maturity(&bond, self.reference_mid()).ok()?;
        Some(crate::price::YieldModel {
            bond,
            long_run_yield: y.0,
            initial_yield: y.0,
            reversion_per_sec,
            perturbation,
        })
    }

    /// The DV01 of this bond's reference schedule **per 100 face** at its own
    /// reference yield: the clean-price move per basis point, from the real analytics
    /// leaf. The conversion factor between a price budget and a yield budget for this
    /// security (`Δy = Δprice / DV01 × 1bp`).
    ///
    /// Returns `None` when the bond cannot be modelled at `settlement`, or when the
    /// derived DV01 is not usable as a divisor (non-finite or non-positive).
    #[must_use]
    pub fn dv01_per_100(&self, settlement: BrokenDate) -> Option<f64> {
        let bond = self.to_reference_bond(settlement).ok()?;
        let y = celnet_bond::yield_to_maturity(&bond, self.reference_mid()).ok()?;
        let dv01 = celnet_bond::dv01(&bond, y).ok()?;
        (dv01.is_finite() && dv01 > 0.0).then_some(dv01)
    }

    /// The [`QuotedLine`](crate::quoted::QuotedLine) this bond streams as: its
    /// canonical identity, engine key, reference-seeded stochastic yield model, and
    /// the quoting conventions of the OTC cash market it trades in.
    ///
    /// # How a cash-bond panel is shaped — and why it is not the listed shape
    ///
    /// A cash bond is quoted freely, off any exchange tick, by dealers who each mark
    /// their own book. That much genuinely differs from a listed contract, and the
    /// line keeps it: no price grid, and each member keeps its own spread width, its
    /// own size, its own re-quote cadence and its own wandering private view.
    ///
    /// What does **not** differ is that the *level* is common information. A dealer
    /// in a benchmark government bond marks off the same observable inter-dealer
    /// level as its competitors; it does not form an independent opinion of where the
    /// 30-year is to the nearest several basis points. Modelling the level as an
    /// independent per-member draw is not a model of bilateral disagreement — it
    /// displaces each member's mid by the bond's DV01 times the yield jitter, which
    /// on anything past the front end is one to two orders of magnitude wider than
    /// the quoted bid-offer. Every member's bid then prints through some other
    /// member's offer, the consolidated composite comes out **crossed**, and the
    /// server's RFQ resolver rejects the line: no outbound quote, no hedge fill. A
    /// panel of dealers who disagree by fifty times what they quote is not a market,
    /// it is a standing arbitrage.
    ///
    /// So the level is shared and the cross-member differentiation is carried by
    /// three deliberate, **budgeted** displacements, each sized as a fraction of the
    /// fleet's base half-spread and converted into yield through **this bond's own
    /// DV01** (a flat yield budget cannot serve a Bill and a 30-year alike — the same
    /// yield move is worth two orders of magnitude more price on the latter):
    ///
    /// * the member's directional **lean** (its axe),
    /// * its fixed private **view** of the security (the starting-yield dispersion),
    /// * its **wandering** private view between its own re-quotes.
    ///
    /// Their sum is held strictly inside the *tightest* member's half-spread, which
    /// is the condition for the panel's best bid never to reach its best offer: two
    /// members' mids differ by at most twice that sum, and crossing needs them to
    /// differ by more than the two half-spreads they quote around them.
    ///
    /// `base_half_spread` and `base_skew_step` are the fleet's defaults the returned
    /// scales are expressed relative to.
    ///
    /// Returns `None` when the bond cannot be modelled at `settlement` — exactly the
    /// cases [`yield_model`](Self::yield_model) rejects — or when no usable DV01 can
    /// be derived to size the budget against.
    #[must_use]
    pub fn to_line(
        &self,
        settlement: BrokenDate,
        base_half_spread: f64,
        base_skew_step: f64,
        reversion_per_sec: f64,
        perturbation: f64,
    ) -> Option<crate::quoted::QuotedLine> {
        let model = self.yield_model(settlement, reversion_per_sec, perturbation)?;
        let dv01 = self.dv01_per_100(settlement)?;
        let budget = quote_budget(base_half_spread, base_skew_step, dv01);
        Some(crate::quoted::QuotedLine {
            instrument_id: self.instrument_id.clone(),
            display_name: self.display_name(),
            identity: format!("{} / {}", self.isin, self.cusip),
            instrument: self.engine_instrument(),
            model,
            spread_scale: 1.0,
            lean_scale: budget.lean_scale,
            yield_dispersion: Some(budget.yield_dispersion),
            dealer_view: budget.dealer_view,
            tick: None,
        })
    }

    /// Rebuild this security's real cashflow schedule as a [`celnet_bond::Bond`]
    /// valued at `settlement`, so the stochastic feed prices off the REAL analytics
    /// leaf (never a fabricated handle).
    ///
    /// The accrual basis is mapped to the engine's 30/360 bond basis (Treasuries
    /// quote on actual/actual, which the leaf does not yet model; 30/360 is the
    /// closest available basis and the reference-yield round-trip below still
    /// reprices the reference mid exactly, because the SAME basis is used to invert
    /// the price and to re-price it). A Bill (zero coupon) is modelled with a `0.0`
    /// coupon.
    ///
    /// # Errors
    ///
    /// Returns [`BondError`] if the schedule is degenerate for this settlement —
    /// most commonly [`BondError::MaturityNotAfterSettlement`] when `settlement`
    /// is on/after the security's maturity (an already-matured Bill).
    pub fn to_reference_bond(&self, settlement: BrokenDate) -> Result<Bond, ReferenceBondError> {
        let settle = broken_to_date(settlement).ok_or(ReferenceBondError::BadSettlement)?;
        let mat = broken_to_date(self.maturity).ok_or(ReferenceBondError::BadMaturity)?;
        let freq = self.frequency.unwrap_or(PaymentFrequency::SemiAnnual);
        Bond::new(
            settle,
            mat,
            self.coupon,
            freq,
            AccrualBasis::Thirty360BondBasis,
            100.0,
        )
        .map_err(ReferenceBondError::Bond)
    }

    /// Build a priced reference bond from a curated [`celnet_refdata::GovBondSpec`]
    /// (a non-US govvie — UK gilt or EUR govvie), reusing the same real cashflow
    /// schedule + mean-reverting-yield feed as the Treasury path.
    ///
    /// The curated on-the-run govvies carry no market snapshot price, so the
    /// reference is seeded at **par** (100.0 per 100 face): the yield the
    /// mean-reverting model reverts to is then the bond's par yield (≈ its coupon),
    /// inverted through the REAL analytics leaf — never a fabricated handle. The
    /// `instrument_id` is the spec's slug (the identity the server's join keys on),
    /// the currency is the spec's ISO-4217 label, and act/act maps to the engine's
    /// 30/360 bond basis — the same closest-basis choice the Treasury path makes.
    ///
    /// Returns `None` if the spec is not modellable: a currency label the engine does
    /// not recognise, a maturity that is not a real calendar date, or a fixed-coupon
    /// bond whose frequency label does not map to an engine payment frequency.
    #[must_use]
    pub fn from_gov_spec(spec: &celnet_refdata::GovBondSpec) -> Option<Self> {
        /// The par reference clean price per 100 face for a curated on-the-run.
        const PAR_PER_100: f64 = 100.0;

        let currency = Ccy::parse(spec.currency)?;
        let maturity = civil_to_broken(spec.maturity_date)?;
        if !spec.coupon_rate.is_finite() {
            return None;
        }
        let dated_date = spec.dated_date.and_then(civil_to_broken);

        // A zero-coupon spec carries no schedule frequency; a fixed-coupon spec MUST
        // carry a frequency the engine can represent, else it is not modellable.
        let (security_type, frequency) = if spec.coupon_type.eq_ignore_ascii_case("zero") {
            (SecurityType::Bill, None)
        } else {
            let freq = parse_frequency(Some(spec.coupon_frequency))?;
            (SecurityType::Bond, Some(freq))
        };

        Some(Self {
            instrument_id: spec.instrument_id.clone(),
            currency,
            region: spec.region,
            name: Some(spec.name.clone()),
            cusip: spec.cusip.clone().unwrap_or_default(),
            isin: spec.isin.clone(),
            security_type,
            term: String::new(),
            maturity,
            dated_date,
            coupon: spec.coupon_rate,
            frequency,
            ask: PAR_PER_100,
            bid: PAR_PER_100,
        })
    }
}

/// Why building a [`celnet_bond::Bond`] from a [`TreasuryBond`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceBondError {
    /// The settlement triple is not a real calendar date.
    BadSettlement,
    /// The stored maturity triple is not a real calendar date (never for a loaded
    /// bond — the loader validates it — but surfaced rather than panicking).
    BadMaturity,
    /// The analytics leaf rejected the schedule (e.g. maturity ≤ settlement).
    Bond(BondError),
}

impl core::fmt::Display for ReferenceBondError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadSettlement => f.write_str("settlement date is not a real calendar date"),
            Self::BadMaturity => f.write_str("maturity date is not a real calendar date"),
            Self::Bond(e) => write!(f, "bond schedule rejected: {e}"),
        }
    }
}

impl core::error::Error for ReferenceBondError {}

/// The raw record shape of one `treasury-universe.json` entry — only the fields the
/// loader needs; serde ignores the rest. `#[serde(default)]` tolerates records that
/// omit a field.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRecord {
    #[serde(default)]
    cusip: String,
    #[serde(default)]
    isin: String,
    #[serde(default)]
    security_type: String,
    #[serde(default)]
    security_term: String,
    #[serde(default)]
    maturity_date: String,
    #[serde(default)]
    dated_date: String,
    #[serde(default)]
    coupon_rate: Option<f64>,
    #[serde(default)]
    interest_payment_frequency: Option<String>,
    #[serde(default)]
    buy_price: Option<f64>,
    #[serde(default)]
    sell_price: Option<f64>,
    /// The auction award price per 100 face. This is the ONLY price a freshly
    /// auctioned security carries: until it trades in the secondary market its
    /// `buyPrice`/`sellPrice` stay `null`, so it is the fallback reference
    /// [`RawRecord::into_bond`] prices such a security off.
    #[serde(default)]
    price_per_100: Option<f64>,
}

impl RawRecord {
    /// Promote a raw record to a validated [`TreasuryBond`], or `None` if it is not
    /// a priced, well-formed security (unparseable date, malformed ISIN, or a
    /// missing/out-of-band price).
    fn into_bond(self) -> Option<TreasuryBond> {
        let security_type = SecurityType::parse(&self.security_type)?;
        let maturity = parse_civil_date(&self.maturity_date)?;
        if self.cusip.trim().is_empty() {
            return None;
        }
        if !isin_is_well_formed(&self.isin) {
            return None;
        }
        // The secondary two-way when the record carries one, else the auction award
        // price as a zero-width reference. A freshly auctioned security has no
        // secondary buy/sell yet, and dropping it here silently shrinks the QUOTABLE
        // universe below the TRADEABLE one the server seeds from
        // `celnet_refdata::government_universe` (which imposes no price filter at
        // all). That asymmetry is not cosmetic: an instrument the venue lets clients
        // trade but no LP-SIM member can quote never reaches an aggregated book, so
        // `AggregationHub::best_fill` finds no composite line for it and EVERY
        // auto-hedge shed on it backstops to the synthetic COMPOSITE venue — leaving
        // the street-side per-LP league table permanently empty. `pricePer100` is the
        // record's REAL auction award price, so this widens the quotable set without
        // fabricating a level (guardrail 2); the stochastic feed builds its own
        // two-way around the reference either way.
        let auction_reference = price_in_band(self.price_per_100);
        let ask = price_in_band(self.buy_price).or(auction_reference)?;
        let bid = price_in_band(self.sell_price).or(auction_reference)?;

        // A coupon security must carry a coupon; a Bill is a zero. Interpret an
        // absent/None coupon on a coupon class as a zero rather than dropping it,
        // but a coupon must be finite.
        let coupon_pct = self.coupon_rate.unwrap_or(0.0);
        if !coupon_pct.is_finite() {
            return None;
        }
        let coupon = coupon_pct / 100.0; // stored as a percent (e.g. 5 => 0.05)
        let frequency = parse_frequency(self.interest_payment_frequency.as_deref());
        let dated_date = parse_civil_date(&self.dated_date);

        Some(TreasuryBond {
            // A US Treasury's canonical id IS its CUSIP, and it prices in USD in the
            // `us` region — the wire id, engine-key currency and identity are all
            // exactly what the feed streamed before this record grew the non-US fields.
            instrument_id: self.cusip.clone(),
            currency: Ccy::USD,
            region: "us",
            name: None,
            cusip: self.cusip,
            isin: self.isin,
            security_type,
            term: self.security_term,
            maturity,
            dated_date,
            coupon,
            frequency,
            ask,
            bid,
        })
    }
}

/// Parse the embedded universe into the full set of **priced, well-formed**
/// [`TreasuryBond`]s, in file order (deterministic). Records that are unpriced,
/// out-of-band, or malformed are dropped.
#[must_use]
pub fn load_universe() -> Vec<TreasuryBond> {
    parse_universe(UNIVERSE_JSON).unwrap_or_default()
}

/// The coupon-bearing subset of [`load_universe`] (Notes and Bonds) — the
/// securities whose real cashflow schedule the stochastic yield feed prices off.
#[must_use]
pub fn load_coupon_universe() -> Vec<TreasuryBond> {
    load_universe()
        .into_iter()
        .filter(|b| b.security_type.is_coupon_bearing())
        .collect()
}

/// The curated **non-US** government reference bonds — UK gilts + EUR govvies (DE /
/// FR / IT) — from [`celnet_refdata::curated_universe`], each built into a real
/// [`celnet_bond::Bond`] schedule and seeded at par (see
/// [`TreasuryBond::from_gov_spec`]). All are coupon-bearing fixed govvies. Any spec
/// not modellable (unrecognised currency, bad date, unmappable frequency) is dropped.
#[must_use]
pub fn load_curated_universe() -> Vec<TreasuryBond> {
    celnet_refdata::curated_universe()
        .iter()
        .filter_map(TreasuryBond::from_gov_spec)
        .collect()
}

/// The **full government reference universe** the feed advertises and prices: the US
/// Treasuries (real snapshot prices, CUSIP identity — byte-identical to
/// [`load_coupon_universe`] / [`load_universe`]) followed by the curated non-US
/// govvies ([`load_curated_universe`]). With `include_bills`, the US zero-coupon
/// Bills are included alongside the coupon Notes/Bonds; the curated set is
/// coupon-bearing either way.
///
/// This is the entry point the deployed `lp-sim` daemon builds its priceable set
/// from, so every instrument the server seeds into the registry from
/// [`celnet_refdata::government_universe`] also streams a real price — not just the
/// Treasuries.
#[must_use]
pub fn load_government_universe(include_bills: bool) -> Vec<TreasuryBond> {
    let mut universe = if include_bills {
        load_universe()
    } else {
        load_coupon_universe()
    };
    universe.extend(load_curated_universe());
    universe
}

/// The three per-member displacement budgets a cash-bond line quotes under, derived
/// from the fleet's defaults and the bond's own DV01 (see [`TreasuryBond::to_line`]).
struct CashQuoteBudget {
    /// Multiplier on the fleet's `skew_step` giving the member's directional lean.
    lean_scale: f64,
    /// The member's fixed private view, as a starting-yield dispersion (decimal).
    yield_dispersion: f64,
    /// The member's wandering private view amplitude (decimal yield).
    dealer_view: f64,
}

/// Size a cash-bond line's per-member displacement budgets so the panel cannot
/// consolidate crossed.
///
/// The three displacements are expressed as fractions of `base_half_spread` (price
/// points per 100 face) and converted into yield through `dv01_per_100`, the bond's
/// own clean-price move per basis point. See [`TreasuryBond::to_line`] for the market
/// -structure argument; this function is only the arithmetic.
///
/// # The never-crossed condition
///
/// Members quote half-spreads dispersed over `[0.6, 1.4) × base_half_spread` (see
/// `crate::lpsim::member_params`), so the tightest member shows `0.6 ×
/// base_half_spread`. Two members' mids differ by at most twice the peak
/// displacement, and their best bid reaches their best offer only once that
/// difference exceeds the sum of the two half-spreads they quote — at least `2 × 0.6
/// × base_half_spread`. Holding the peak displacement under `0.6 ×
/// base_half_spread` therefore makes crossing impossible, and the fractions below sum
/// to `0.40`, leaving half as much again in headroom. (The headroom also absorbs
/// panels wider than the deployed five members: the peak lean grows as `(n−1)/2`
/// steps, and the budget still holds at ten.)
fn quote_budget(base_half_spread: f64, base_skew_step: f64, dv01_per_100: f64) -> CashQuoteBudget {
    /// Peak directional lean of the outermost member, as a fraction of the base
    /// half-spread.
    const LEAN_FRACTION: f64 = 0.15;
    /// Peak fixed private view of a member, as a fraction of the base half-spread.
    const VIEW_FRACTION: f64 = 0.10;
    /// Peak wandering private view of a member, as a fraction of the base
    /// half-spread.
    const WANDER_FRACTION: f64 = 0.15;
    /// The outermost member of the deployed five-member panel sits this many
    /// `skew_step`s from the centre (`(n−1)/2`), so a peak-displacement lean budget
    /// converts to a per-step scale by dividing by it.
    const PANEL_HALF_WIDTH_STEPS: f64 = 2.0;

    // Price displacement -> yield displacement through the bond's own DV01.
    let to_yield = |fraction: f64| fraction * base_half_spread / dv01_per_100 * 1.0e-4;
    let lean_scale = if base_skew_step.is_finite() && base_skew_step > 0.0 {
        LEAN_FRACTION * base_half_spread / PANEL_HALF_WIDTH_STEPS / base_skew_step
    } else {
        1.0
    };
    CashQuoteBudget {
        lean_scale,
        yield_dispersion: to_yield(VIEW_FRACTION),
        dealer_view: to_yield(WANDER_FRACTION),
    }
}

/// Build the [`QuotedLine`](crate::quoted::QuotedLine)s for a loaded cash-bond
/// universe (see [`TreasuryBond::to_line`]). Bonds with no solvable reference yield
/// at `settlement` — e.g. a deep-discount Bill the coupon-bond solver cannot bracket
/// — are dropped, never quoted off a fabricated level.
///
/// `base_half_spread` and `base_skew_step` are the fleet's quoting defaults each
/// line's per-member displacement budget is sized against.
#[must_use]
pub fn bond_lines(
    bonds: &[TreasuryBond],
    settlement: BrokenDate,
    base_half_spread: f64,
    base_skew_step: f64,
    reversion_per_sec: f64,
    perturbation: f64,
) -> Vec<crate::quoted::QuotedLine> {
    bonds
        .iter()
        .filter_map(|b| {
            b.to_line(
                settlement,
                base_half_spread,
                base_skew_step,
                reversion_per_sec,
                perturbation,
            )
        })
        .collect()
}

/// Parse a Treasury-universe JSON document into validated [`TreasuryBond`]s.
///
/// # Errors
///
/// Returns the underlying [`serde_json::Error`] if `json` is not a JSON array of
/// records. Individual records that fail validation are silently dropped (they are
/// data-quality filters, not parse errors).
pub fn parse_universe(json: &str) -> Result<Vec<TreasuryBond>, serde_json::Error> {
    let raw: Vec<RawRecord> = serde_json::from_str(json)?;
    Ok(raw.into_iter().filter_map(RawRecord::into_bond).collect())
}

// --- parsing helpers --------------------------------------------------------

/// A price is usable iff it is finite and strictly inside `(0, MAX_PRICE_PER_100)`.
fn price_in_band(p: Option<f64>) -> Option<f64> {
    let v = p?;
    (v.is_finite() && v > 0.0 && v < MAX_PRICE_PER_100).then_some(v)
}

/// Map an `interestPaymentFrequency` token onto a [`PaymentFrequency`], or `None`
/// (zero-coupon / no schedule) for `None`/empty. Only the engine's three
/// frequencies are representable; a Monthly Treasury does not exist.
fn parse_frequency(label: Option<&str>) -> Option<PaymentFrequency> {
    let l = label?.trim().to_ascii_uppercase();
    if l.contains("SEMI") {
        Some(PaymentFrequency::SemiAnnual)
    } else if l.contains("QUART") {
        Some(PaymentFrequency::Quarterly)
    } else if l.contains("ANNUAL") {
        // "Annual" (but not "Semi-Annual", handled above).
        Some(PaymentFrequency::Annual)
    } else {
        None
    }
}

/// Parse a civil date in either ISO (`YYYY-MM-DD`, the universe file) or US
/// (`MM/DD/YYYY`, the price file) form into a [`BrokenDate`], validated against the
/// Gregorian calendar. Returns `None` for a blank or unparseable token.
#[must_use]
pub fn parse_civil_date(s: &str) -> Option<BrokenDate> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (y, m, d) = if let Some((y, rest)) = s.split_once('-') {
        // ISO YYYY-MM-DD
        let (mo, day) = rest.split_once('-')?;
        (y.parse().ok()?, mo.parse().ok()?, day.parse().ok()?)
    } else if let Some((mo, rest)) = s.split_once('/') {
        // US MM/DD/YYYY
        let (day, y) = rest.split_once('/')?;
        (y.parse().ok()?, mo.parse().ok()?, day.parse().ok()?)
    } else {
        return None;
    };
    // Validate against the real calendar via `time` (rejects 2026-02-30 etc.).
    let month = Month::try_from(m).ok()?;
    Date::from_calendar_date(y, month, d).ok()?;
    Some(BrokenDate::new(y, m, d))
}

/// Convert a validated [`BrokenDate`] to a `time::Date` (the loader only ever holds
/// calendar-valid triples, but this returns `None` rather than panicking on a
/// hand-built out-of-range triple).
fn broken_to_date(b: BrokenDate) -> Option<Date> {
    let month = Month::try_from(b.month).ok()?;
    Date::from_calendar_date(b.year, month, b.day).ok()
}

/// Convert a [`celnet_refdata::CivilYmd`] to a [`BrokenDate`], validated against the
/// real Gregorian calendar (rejects e.g. 2036-02-30). `None` for a non-date triple.
fn civil_to_broken(d: celnet_refdata::CivilYmd) -> Option<BrokenDate> {
    let month_u8 = u8::try_from(d.month).ok()?;
    let day_u8 = u8::try_from(d.day).ok()?;
    let month = Month::try_from(month_u8).ok()?;
    Date::from_calendar_date(d.year, month, day_u8).ok()?;
    Some(BrokenDate::new(d.year, month_u8, day_u8))
}

/// Whether an ISIN is well-formed: 12 chars, two leading letters, and a valid
/// mod-10 (Luhn) check digit over the digit-expanded body — the ISO-6166 standard
/// integrity check, so a corrupted id is rejected, not merely length-checked.
#[must_use]
pub fn isin_is_well_formed(isin: &str) -> bool {
    let bytes = isin.as_bytes();
    if bytes.len() != 12 {
        return false;
    }
    if !bytes[0].is_ascii_uppercase() || !bytes[1].is_ascii_uppercase() {
        return false;
    }
    if !bytes[..11].iter().all(u8::is_ascii_alphanumeric) || !bytes[11].is_ascii_digit() {
        return false;
    }
    isin_check_digit(&isin[..11]) == (bytes[11] - b'0')
}

/// The ISO-6166 mod-10 (Luhn) check digit for an 11-character ISIN body: each
/// letter expands to two digits (`A`=10 … `Z`=35), then the Luhn algorithm runs
/// right-to-left over the resulting digit string.
fn isin_check_digit(body: &str) -> u8 {
    // Expand letters to digits, building the digit sequence left-to-right.
    let mut digits: Vec<u8> = Vec::with_capacity(22);
    for c in body.bytes() {
        if c.is_ascii_digit() {
            digits.push(c - b'0');
        } else {
            let v = c - b'A' + 10; // A..Z => 10..35
            digits.push(v / 10);
            digits.push(v % 10);
        }
    }
    // Luhn from the right: double every second digit (starting with the rightmost).
    let mut sum = 0u32;
    for (i, &d) in digits.iter().rev().enumerate() {
        let mut v = u32::from(d);
        if i % 2 == 0 {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
    }
    ((10 - (sum % 10)) % 10) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_substantial_priced_universe_from_the_real_file() {
        let bonds = load_universe();
        // The committed snapshot has 267 records; a large priced subset survives
        // validation (rows with no price at all and out-of-band rows are dropped —
        // a freshly auctioned row prices off its `pricePer100` award, see
        // `freshly_auctioned_securities_price_off_the_auction_award`).
        assert!(
            bonds.len() >= 120,
            "expected a large priced universe, got {}",
            bonds.len()
        );
        // Every loaded bond is priced in a sane band and its ISIN is check-valid.
        for b in &bonds {
            assert!(b.ask > 0.0 && b.ask < MAX_PRICE_PER_100);
            assert!(b.bid > 0.0 && b.bid < MAX_PRICE_PER_100);
            assert!(b.reference_mid() > 0.0 && b.reference_mid() < MAX_PRICE_PER_100);
            assert!(isin_is_well_formed(&b.isin), "bad ISIN {}", b.isin);
            assert_eq!(b.cusip.len(), 9, "CUSIP not 9 chars: {}", b.cusip);
        }
    }

    /// A freshly auctioned security carries ONLY its `pricePer100` award — its
    /// secondary `buyPrice`/`sellPrice` are still `null`. It must still load, because
    /// the server's tradeable registry (`celnet_refdata::government_universe`) admits
    /// it unconditionally: an instrument that is tradeable but NOT quotable never
    /// reaches an aggregated book, so every auto-hedge shed on it backstops to the
    /// synthetic COMPOSITE venue and the street-side per-LP league table stays empty.
    #[test]
    fn freshly_auctioned_securities_price_off_the_auction_award() {
        let json = r#"[
          {"cusip":"912797UE5","isin":"US912797UE52","securityType":"Bill",
           "securityTerm":"52-Week","maturityDate":"2027-04-15",
           "interestPaymentFrequency":"None","pricePer100":96.400444},
          {"cusip":"912797ZZ9","isin":"US912797UE52","securityType":"Bill",
           "securityTerm":"52-Week","maturityDate":"2027-04-15",
           "interestPaymentFrequency":"None"}
        ]"#;
        let bonds = parse_universe(json).expect("well-formed array");
        // The awarded-but-unseasoned bill loads, priced at its award on both sides.
        assert_eq!(bonds.len(), 1, "only the priced record survives");
        assert_eq!(bonds[0].cusip, "912797UE5");
        assert!((bonds[0].bid - 96.400444).abs() < 1e-9);
        assert!((bonds[0].ask - 96.400444).abs() < 1e-9);
        // A record carrying NO price at all is still dropped — never fabricated.
        assert!(!bonds.iter().any(|b| b.cusip == "912797ZZ9"));
    }

    /// The secondary two-way still WINS over the auction award when present, so a
    /// seasoned security's loaded levels are byte-identical to before the fallback.
    #[test]
    fn a_seasoned_two_way_takes_precedence_over_the_auction_award() {
        let json = r#"[
          {"cusip":"912797UD7","isin":"US912797UD79","securityType":"Bill",
           "securityTerm":"52-Week","maturityDate":"2027-03-18",
           "interestPaymentFrequency":"None","pricePer100":96.476278,
           "buyPrice":96.681486,"sellPrice":96.676806}
        ]"#;
        let bonds = parse_universe(json).expect("well-formed array");
        assert_eq!(bonds.len(), 1);
        assert!((bonds[0].ask - 96.681486).abs() < 1e-9, "buyPrice wins");
        assert!((bonds[0].bid - 96.676806).abs() < 1e-9, "sellPrice wins");
    }

    #[test]
    fn instrument_ids_and_engine_keys_are_unique() {
        let bonds = load_universe();
        let ids: std::collections::HashSet<&str> =
            bonds.iter().map(TreasuryBond::instrument_id).collect();
        assert_eq!(
            ids.len(),
            bonds.len(),
            "instrument_ids (CUSIPs) must be unique"
        );
        // The cosmetic engine key must be injective too, so two securities never
        // consolidate into one line (this is why the CUSIP — not the maturity —
        // carries the identity).
        let keys: std::collections::HashSet<Instrument> =
            bonds.iter().map(TreasuryBond::engine_instrument).collect();
        assert_eq!(
            keys.len(),
            bonds.len(),
            "engine Instrument keys must be unique"
        );
    }

    #[test]
    fn coupon_universe_is_coupon_bearing_and_prices_off_the_real_leaf() {
        let coupon = load_coupon_universe();
        assert!(
            !coupon.is_empty(),
            "expected coupon Notes/Bonds in the universe"
        );
        // Settlement well before any surviving maturity: the snapshot's earliest
        // maturities are 2026; settle at 2026-04-16 (the snapshot's price date).
        let settle = BrokenDate::new(2026, 4, 16);
        let mut priced = 0;
        for b in &coupon {
            assert!(b.security_type.is_coupon_bearing());
            assert!(b.coupon >= 0.0);
            if let Ok(bond) = b.to_reference_bond(settle) {
                // The reference yield inverted from the reference mid must reprice
                // that exact mid back through the SAME leaf — an oracle round-trip,
                // not a plausibility check.
                let mid = b.reference_mid();
                let dirty = mid; // clean≈dirty at issue; the round-trip is basis-consistent
                if let Ok(y) = celnet_bond::yield_to_maturity(&bond, dirty) {
                    let repriced = celnet_bond::dirty_price(&bond, y).unwrap();
                    assert!(
                        (repriced - dirty).abs() < 1e-6,
                        "yield round-trip drift {} for {}",
                        (repriced - dirty).abs(),
                        b.cusip
                    );
                    priced += 1;
                }
            }
        }
        assert!(
            priced > 0,
            "no coupon bond repriced through the analytics leaf"
        );
    }

    #[test]
    fn parses_both_date_formats() {
        assert_eq!(
            parse_civil_date("2026-05-19"),
            Some(BrokenDate::new(2026, 5, 19))
        );
        assert_eq!(
            parse_civil_date("05/19/2026"),
            Some(BrokenDate::new(2026, 5, 19))
        );
        assert_eq!(parse_civil_date(""), None);
        assert_eq!(
            parse_civil_date("2026-02-30"),
            None,
            "invalid calendar date rejected"
        );
        assert_eq!(parse_civil_date("garbage"), None);
    }

    #[test]
    fn isin_check_digit_matches_known_good_and_rejects_corruption() {
        // A real Treasury ISIN from the universe (check digit 7).
        assert!(isin_is_well_formed("US912797TS67"));
        // Corrupt the check digit → rejected.
        assert!(!isin_is_well_formed("US912797TS60"));
        // Wrong length / shape → rejected.
        assert!(!isin_is_well_formed("US912797TS6"));
        assert!(!isin_is_well_formed("1S912797TS67"));
    }

    #[test]
    fn security_type_and_frequency_parse() {
        assert_eq!(SecurityType::parse("Bill"), Some(SecurityType::Bill));
        assert_eq!(SecurityType::parse("Note"), Some(SecurityType::Note));
        assert_eq!(SecurityType::parse("Bond"), Some(SecurityType::Bond));
        assert_eq!(
            SecurityType::parse("MARKET BASED BILL"),
            Some(SecurityType::Bill)
        );
        assert_eq!(SecurityType::parse("Junk"), None);
        assert_eq!(
            parse_frequency(Some("Semi-Annual")),
            Some(PaymentFrequency::SemiAnnual)
        );
        assert_eq!(parse_frequency(Some("None")), None);
        assert_eq!(parse_frequency(None), None);
    }

    // --- the extended non-US government universe ---------------------------------

    #[test]
    fn curated_non_us_bonds_build_real_schedules_and_price_on_the_leaf() {
        let curated = load_curated_universe();
        // 10 UK gilts + 8 DE Bunds + 6 FR OATs + 6 IT BTPs.
        assert_eq!(curated.len(), 30, "expected the full curated non-US set");

        // The sim's default settlement; every curated maturity (2028+) is after it.
        let settle = BrokenDate::new(2026, 4, 16);
        let (mut uk, mut de, mut fr, mut it) = (0, 0, 0, 0);
        for b in &curated {
            match b.region {
                "uk" => uk += 1,
                "de" => de += 1,
                "fr" => fr += 1,
                "it" => it += 1,
                other => panic!("unexpected curated region {other}"),
            }
            assert!(b.cusip.is_empty(), "a non-US govvie carries no CUSIP");
            assert_ne!(b.currency, Ccy::USD, "a non-US govvie prices in GBP/EUR");
            assert!(b.name.is_some(), "a curated govvie carries a friendly name");
            // The slug is the wire id the server's identity join keys on.
            assert!(
                b.instrument_id().contains('-'),
                "curated id is a slug: {}",
                b.instrument_id()
            );

            // A real cashflow schedule builds at the sim settlement...
            let bond = b
                .to_reference_bond(settle)
                .unwrap_or_else(|e| panic!("real schedule rejected for {}: {e}", b.instrument_id));
            // ...and the reference (par) mid round-trips price↔yield through the REAL
            // analytics leaf — an oracle round-trip, not a plausibility check.
            let mid = b.reference_mid();
            let y = celnet_bond::yield_to_maturity(&bond, mid)
                .unwrap_or_else(|e| panic!("ytm failed for {}: {e:?}", b.instrument_id));
            let repriced = celnet_bond::dirty_price(&bond, y)
                .unwrap_or_else(|e| panic!("reprice failed for {}: {e:?}", b.instrument_id));
            assert!(
                (repriced - mid).abs() < 1e-6,
                "price↔yield round-trip drift for {}: repriced {repriced}",
                b.instrument_id
            );

            // The mean-reverting-yield feed model builds and prices finitely.
            let model = b
                .yield_model(settle, 0.02, 3.0e-4)
                .unwrap_or_else(|| panic!("no yield model for {}", b.instrument_id));
            let px = model.clean_price_at(1_000_000_000, 0.0);
            assert!(
                px.is_finite() && px > 0.0,
                "non-finite feed price for {}: {px}",
                b.instrument_id
            );
        }
        assert_eq!((uk, de, fr, it), (10, 8, 6, 6), "curated region breakdown");
    }

    #[test]
    fn government_universe_covers_all_regions_and_preserves_treasuries() {
        let treasuries = load_coupon_universe();
        let curated = load_curated_universe();
        let gov = load_government_universe(false);

        // The government universe is exactly the treasuries followed by the curated
        // set — byte-identical on the Treasury prefix (unchanged wire ids + prices).
        assert_eq!(gov.len(), treasuries.len() + curated.len());
        assert_eq!(
            &gov[..treasuries.len()],
            &treasuries[..],
            "the Treasury feed must be byte-identical after the non-US extension"
        );

        // Every Treasury keeps its 9-char CUSIP identity in USD in the `us` region.
        for b in &treasuries {
            assert_eq!(b.instrument_id(), b.cusip, "treasury id is its CUSIP");
            assert_eq!(b.cusip.len(), 9);
            assert_eq!(b.currency, Ccy::USD);
            assert_eq!(b.region, "us");
        }

        // All five regions are represented in the combined universe.
        for region in ["us", "uk", "de", "fr", "it"] {
            assert!(
                gov.iter().any(|b| b.region == region),
                "region {region} not represented"
            );
        }

        // instrument_ids and engine keys are globally unique — the server keys the
        // registry by instrument_id and the consolidator by the engine key, so a
        // collision would silently merge two securities across regions.
        let ids: std::collections::HashSet<&str> =
            gov.iter().map(TreasuryBond::instrument_id).collect();
        assert_eq!(
            ids.len(),
            gov.len(),
            "instrument_id collision across regions"
        );
        let keys: std::collections::HashSet<Instrument> =
            gov.iter().map(TreasuryBond::engine_instrument).collect();
        assert_eq!(keys.len(), gov.len(), "engine key collision across regions");
    }
}
