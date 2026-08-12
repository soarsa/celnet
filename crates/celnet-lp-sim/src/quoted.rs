//! [`QuotedLine`] — the one thing an LP-SIM member streams a two-way for — and
//! [`MarketStructure`], the market it trades in.
//!
//! The simulator quotes two different kinds of instrument (cash government bonds and
//! listed Treasury futures), and every layer above the price model treats them
//! identically: the book resolver intersects an aggregated book's scope with the set
//! of `instrument_id`s the sim can price, the streaming round emits one `LpQuote` per
//! `(member, instrument_id)` pair, and the consolidator folds them by engine key. A
//! `QuotedLine` is exactly that common shape — an identity, an engine key, a
//! stochastic mid model, and the structure of the market it trades in.
//!
//! Keeping one line type is not a tidiness argument. The sim's **quotable** set is
//! what reaches an aggregated book, and the server's **tradeable** registry is seeded
//! from the same `celnet-refdata` universes. Anything tradeable but not quotable never
//! reaches a book, so `AggregationHub::best_fill` misses it and every hedge shed on it
//! backstops to the synthetic COMPOSITE venue — the exact asymmetry that already bit
//! the freshly-auctioned Treasuries (see `universe::RawRecord::into_bond`). Routing
//! both universes through one line type is what keeps the two sets in step.
//!
//! What is *not* common between them is the market structure, and that is carried
//! explicitly rather than emulated: see [`MarketStructure`].

use celnet_aggregation::Instrument;

use crate::price::YieldModel;

/// The structure of the market an instrument trades in — the thing that decides what
/// a panel of members quoting it actually looks like.
///
/// This is a real distinction, not a parameterisation of one shape. A dealer panel and
/// an order book differ in *what a member is*: on the OTC side a member is a
/// counterparty with its own private view and its own width, and the "market" is the
/// envelope of their bilateral two-ways; on a listed venue a member is a participant
/// in **one** book, sees the same top of book as every other participant, and differs
/// only in how much size it is working there. Modelling the second as a tightened
/// version of the first is what produces the crossed composites and the fractional
/// lot sizes that make a futures hedge unfillable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarketStructure {
    /// **Bilateral OTC dealing** — the cash government-bond feed.
    ///
    /// Each member forms its own two-way around its own view of the level: it leans
    /// deliberately, carries a fixed private mark and a wandering one, and quotes its
    /// own width. All three displacements are budgeted by the caller to stay inside
    /// the tightest member's half-spread, because a market whose dealers disagree by
    /// more than they quote is not a market, it is an arbitrage.
    Bilateral {
        /// A multiplier on the quoting member's half-spread. `1.0` leaves it untouched.
        spread_scale: f64,
        /// A multiplier on the member's directional skew — how far its market leans
        /// away from the panel's centre.
        spread_lean_scale: f64,
        /// This instrument's own per-member **starting**-yield dispersion (decimal
        /// yield): the fixed part of a dealer's private view of where the security is.
        /// `None` falls back to the fleet's
        /// [`yield_dispersion`](crate::LpSimConfig::yield_dispersion).
        ///
        /// A single yield dispersion cannot serve instruments of different duration:
        /// the price displacement it produces is the instrument's DV01 times that
        /// yield move, so the same 1.5 bp is a fraction of a cent on a Bill and more
        /// than a full price point on a 30-year bond. Every real line therefore sets
        /// its own, sized in **price** terms off its own derived DV01.
        starting_yield_dispersion: Option<f64>,
        /// This instrument's per-member **time-varying** private-view amplitude
        /// (decimal yield): how far a member's own mark wanders from the panel-common
        /// market level between its own re-quotes.
        dealer_view: f64,
    },
    /// **A central limit order book** — the listed Treasury futures venue.
    ///
    /// # What this models
    ///
    /// * **One price for everyone.** There is a single order book, so there is a
    ///   single best bid and best offer, and every member of the panel quotes exactly
    ///   it. No per-member lean, no per-member width, no per-member private mark on
    ///   the level: an exchange print reaches every participant at once. This is why a
    ///   listed composite is uncrossed **by construction** rather than by a dispersion
    ///   budget that has to be re-verified whenever a knob moves.
    /// * **Firm, on-grid quotes.** Both sides are snapped onto the contract's
    ///   published minimum price increment (bid down, offer up), so the two-way is
    ///   always a whole number of ticks wide and can never invert. There is no last
    ///   look: an order that crosses the book is filled at the book's price.
    /// * **Whole-contract lots.** A futures market deals in contracts, not in
    ///   arbitrary face amounts. The size a member shows is a whole multiple of the
    ///   contract's face value, so a `bid_size` read back as face and divided by
    ///   [`lot_face`](Self::CentralLimitOrderBook::lot_face) is always an integer.
    /// * **Members differ in size, not price.** Price-time priority means competition
    ///   at the top of an order book is over quantity and queue position. The panel's
    ///   members therefore differ only in how many contracts each is working — which
    ///   is what the per-member firm size on the quote carries.
    ///
    /// # What this deliberately does not model
    ///
    /// * **Depth beyond the top of book.** The feed seam is a top-of-book
    ///   [`VenueQuote`](celnet_aggregation::VenueQuote); there are no further price
    ///   levels, so an order larger than the shown size has no ladder to walk.
    /// * **Queue position and time priority as an observable.** Members differ in
    ///   working size, but the order in which they would be filled at a common price
    ///   is not published, and the consolidator's tie-break (first improving member in
    ///   deterministic id order) stands in for it.
    /// * **The delivery option and the live cheapest-to-deliver.** The level is the
    ///   notional deliverable's price (see the `futures` module); a basis trader would
    ///   see no net-basis dynamics.
    /// * **Auction states, limits and halts.** No opening/closing auction, no daily
    ///   price limit, no trading halt: the book quotes continuously.
    /// * **Margin, clearing and settlement variation.** The venue publishes prices;
    ///   the initial/variation-margin lifecycle of a cleared contract is not simulated.
    CentralLimitOrderBook {
        /// The contract's published minimum price increment for an outright trade, in
        /// points of 100 face. Both sides are snapped onto this grid.
        tick: f64,
        /// The half-spread of the exchange's top of book, in points — identical for
        /// every member, because there is one book. After grid snapping this prints a
        /// one- or two-tick market, which is how these contracts trade.
        half_spread: f64,
        /// The face value of one contract. Every quoted size is a whole multiple of it.
        lot_face: f64,
    },
}

impl MarketStructure {
    /// The minimum price increment this market trades on, if it has a grid.
    #[must_use]
    pub fn tick(&self) -> Option<f64> {
        match self {
            Self::Bilateral { .. } => None,
            Self::CentralLimitOrderBook { tick, .. } => Some(*tick),
        }
    }

    /// Whether this is an exchange-style order book (one price for every member).
    #[must_use]
    pub fn is_order_book(&self) -> bool {
        matches!(self, Self::CentralLimitOrderBook { .. })
    }
}

/// One instrument the simulator can stream a two-way for.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotedLine {
    /// The canonical server `instrument_id` — a Treasury's CUSIP, a curated govvie's
    /// slug, or a futures contract code. The identity the aggregated-book scope, the
    /// wire and the registry all key on.
    pub instrument_id: String,
    /// A short human-friendly blotter/GUI name.
    pub display_name: String,
    /// The cross-reference identity a subscriber renders alongside the name (a bond's
    /// `ISIN / CUSIP`, a future's contract code).
    pub identity: String,
    /// The aggregation engine's asset-agnostic key. Injective over `instrument_id`,
    /// so two distinct instruments never consolidate onto one line.
    pub instrument: Instrument,
    /// The seeded mean-reverting-yield model whose sampled yield is priced to this
    /// instrument's quoted price through the real `celnet-bond` analytics leaf.
    pub model: YieldModel,
    /// The structure of the market this instrument trades in, carrying that market's
    /// quoting conventions. See [`MarketStructure`].
    pub structure: MarketStructure,
}

impl QuotedLine {
    /// The canonical server `instrument_id`.
    #[must_use]
    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }

    /// Whether this line trades on an exchange-style order book.
    #[must_use]
    pub fn is_listed(&self) -> bool {
        self.structure.is_order_book()
    }
}
