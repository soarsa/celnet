//! [`QuotedLine`] — the one thing an LP-SIM member streams a two-way for.
//!
//! The simulator quotes two different kinds of instrument (cash government bonds and
//! listed Treasury futures), and every layer above the price model treats them
//! identically: the book resolver intersects an aggregated book's scope with the set
//! of `instrument_id`s the sim can price, the streaming round emits one `LpQuote` per
//! `(member, instrument_id)` pair, and the consolidator folds them by engine key. A
//! `QuotedLine` is exactly that common shape — an identity, an engine key, a
//! stochastic mid model, and the quoting conventions of the instrument's market.
//!
//! Keeping one line type is not a tidiness argument. The sim's **quotable** set is
//! what reaches an aggregated book, and the server's **tradeable** registry is seeded
//! from the same `celnet-refdata` universes. Anything tradeable but not quotable never
//! reaches a book, so `AggregationHub::best_fill` misses it and every hedge shed on it
//! backstops to the synthetic COMPOSITE venue — the exact asymmetry that already bit
//! the freshly-auctioned Treasuries (see `universe::RawRecord::into_bond`). Routing
//! both universes through one line type is what keeps the two sets in step.

use celnet_aggregation::Instrument;

use crate::price::MidSource;

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
    /// How this line's ground-truth mid is formed. A cash bond / listed future uses
    /// the seeded mean-reverting **yield** priced through the real `celnet-bond`
    /// analytics leaf; a swap/OIS curve point uses the seeded mean-reverting **par
    /// rate** quoted directly (an OIS market quotes a rate, not a price per 100).
    /// Keeping the mid polymorphic here is what lets one line type — and therefore one
    /// aggregated book — carry both quoting conventions.
    pub mid: MidSource,
    /// A multiplier on the quoting LP's half-spread for this instrument. Markets
    /// quote in very different units: a cash bond's two-way is a couple of basis
    /// points of a ~100 price handle, whereas a Treasury future trades a single
    /// minimum price increment wide. `1.0` leaves the LP's own half-spread untouched.
    pub spread_scale: f64,
    /// A multiplier on the quoting LP's directional skew — how far each member's
    /// market leans away from the panel's centre — and, in the same units, on the
    /// per-member dispersion of the starting yield.
    ///
    /// Separate from [`spread_scale`](Self::spread_scale) because the two are not
    /// proportional across market structures. On a listed venue the members' markets
    /// must stay well inside one half-spread of each other, or the panel's best bid
    /// prints through its best offer and the composite comes out **crossed** — which
    /// the server's RFQ resolver rejects, silently starving every hedge routed at it.
    pub lean_scale: f64,
    /// This instrument's own per-member **starting**-yield dispersion (decimal
    /// yield) — the fixed part of a dealer's private view of where the security is,
    /// overriding the fleet default. `None` uses the fleet's
    /// [`yield_dispersion`](crate::LpSimConfig::yield_dispersion).
    ///
    /// A single yield dispersion cannot serve instruments of different duration: the
    /// price displacement it produces is the instrument's DV01 times that yield move,
    /// so the same 1.5 bp of dispersion is a fraction of a cent on a Bill and more
    /// than a full price point on a 30-year bond. Every real line therefore sets its
    /// own, sized in **price** terms off its own derived DV01 and budgeted inside the
    /// tightest member's half-spread.
    pub yield_dispersion: Option<f64>,
    /// This instrument's per-member **time-varying** private-view amplitude (decimal
    /// yield): how far a member's own mark wanders from the panel-common market level
    /// between its own re-quotes (see
    /// [`InstrumentModel::dealer_view`](crate::lp::InstrumentModel::dealer_view)).
    ///
    /// Sized off the same DV01 budget as
    /// [`yield_dispersion`](Self::yield_dispersion), and for the same reason: the
    /// three per-member displacements (deliberate lean, fixed private view, wandering
    /// private view) must together stay strictly inside the tightest member's
    /// half-spread, or the panel's best bid prints through its best offer and the
    /// composite comes out crossed.
    ///
    /// `0.0` where a wandering sub-increment view is not meaningful — a listed
    /// contract quoted on a tick grid snaps it away, and its makers compete on size
    /// and queue position rather than on a private mark.
    pub dealer_view: f64,
    /// The minimum price increment this instrument trades on, if it trades on a grid.
    /// A bid is snapped **down** to the grid and an offer **up**, which is what an
    /// exchange-listed market maker does and can never invert a two-way. `None` for
    /// an OTC instrument quoted off-grid (the cash bonds).
    pub tick: Option<f64>,
}

impl QuotedLine {
    /// The canonical server `instrument_id`.
    #[must_use]
    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }
}
