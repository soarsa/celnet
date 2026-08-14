//! The **simulated counterparty roster** — the single definition of who the
//! simulated liquidity providers are and how each one prices.
//!
//! Before this module the panel was `LP-SIM-01`…`LP-SIM-0N`: an anonymous numbered
//! fleet whose members differed only by four seeded draws off a child seed. Two
//! consequences, both operator-visible:
//!
//! 1. **Fill attribution was meaningless.** A booked hedge stamps the winning
//!    member's venue id into `HedgeProvenance.lp_won`, which the blotter and the LP
//!    panel render verbatim. `LP-SIM-03` tells a trader nothing about *who* filled.
//! 2. **The members were not really distinguishable.** Their characters came from
//!    the same uniform draw over the same narrow band, so the panel was four
//!    near-copies of one curve and the "winner" of the consolidated best bid/offer
//!    was noise rather than a persistent liquidity characteristic.
//!
//! This module replaces both with a **roster**: a fixed, ordered set of named
//! simulated counterparties, each carrying a stable id, a display name, a venue
//! archetype, and an explicit pricing personality (spread, directional axe, size
//! appetite, quoted depth, response latency, refresh cadence, self-reported
//! quality). Every member of every simulator panel is drawn from here — there are
//! no member-shaped literals anywhere else in the crate.
//!
//! # These names are simulated counterparty identities, not product identifiers
//!
//! Guardrail #8 forbids vendor/competitor names in **product** artifacts (crates,
//! modules, types, traits, functions). It does not forbid *test data*: a simulated
//! counterparty must be named like a counterparty or the simulation teaches the
//! operator nothing. Every id therefore carries a mandatory `-sim` suffix so a name
//! appearing in a blotter, an LP panel or a provenance record is unmistakably
//! synthetic, and the suffix is asserted by [`tests::every_id_is_marked_simulated`]
//! so a future edit cannot quietly drop it. The API identifiers in this file
//! ([`SimLpProfile`], [`VenueArchetype`], [`roster`]) stay purpose-named.
//!
//! # Determinism
//!
//! A profile is a set of **constants**, not draws: the same roster always produces
//! the same characters, and a member's identity is decoupled from the run seed. The
//! run seed still decorrelates the *price paths* (each member gets its own child
//! seed) and a small seeded jitter is layered on the constants so two runs at
//! different seeds are not pixel-identical — but a member's persistent character
//! (who is tight, who is axed, who shows size) is a property of the roster and is
//! reproducible across every run, which is exactly what makes the LP panel readable.

use crate::rng::unit01;

/// The market-structure archetype a simulated counterparty is modelled on.
///
/// This is not decoration: it determines the shape of the quoted depth and the
/// latency band, which are the two things that make different LPs fill different
/// amounts of the same order (see [`crate::execution`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VenueArchetype {
    /// An electronic all-to-all trading platform: the tightest top-of-book, modest
    /// size at the touch, several thin levels behind it, and the lowest latency.
    ElectronicPlatform,
    /// An electronic request-for-quote platform: a slightly wider but much larger
    /// firm market, shallow behind the touch (a platform quote is the whole clip).
    RequestForQuotePlatform,
    /// A bank principal dealer: the widest market, the largest appetite, deep
    /// behind the touch, and the slowest to respond (a human-supervised book).
    PrincipalDealer,
    /// A listed derivatives exchange: a market pinned one minimum price increment
    /// wide with very deep, slowly-decaying size, quoted in whole contracts.
    ListedExchange,
}

impl VenueArchetype {
    /// A short stable label for logs and operator output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            VenueArchetype::ElectronicPlatform => "electronic_platform",
            VenueArchetype::RequestForQuotePlatform => "rfq_platform",
            VenueArchetype::PrincipalDealer => "principal_dealer",
            VenueArchetype::ListedExchange => "listed_exchange",
        }
    }
}

/// The mandatory suffix marking an id as a simulated counterparty rather than a
/// real one. Enforced by [`SimLpProfile::is_marked_simulated`].
pub const SIM_SUFFIX: &str = "-sim";

/// One simulated liquidity provider's stable identity and pricing personality.
///
/// Every field is a **relative** knob applied to the fleet's own base parameters
/// (`LpSimConfig::half_spread` / `size` / `skew_step`), so one roster serves every
/// asset the simulators quote — a cash bond, an OIS point and a listed future all
/// scale the same personality onto their own market's natural width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimLpProfile {
    /// The stable connection id — the `VenueId` a contribution, a composite leg and
    /// a booked fill's `lp_won` are all attributed to. Always `-sim`-suffixed.
    pub id: &'static str,
    /// The human-readable name a blotter / LP panel renders.
    pub display_name: &'static str,
    /// The market-structure archetype this counterparty is modelled on.
    pub archetype: VenueArchetype,
    /// Multiplier on the fleet's base half-spread. `< 1` quotes tighter than the
    /// fleet base, `> 1` wider.
    pub spread_multiple: f64,
    /// Signed directional axe in units of the fleet's `skew_step`. Positive lifts
    /// both sides (axed to sell — a rich two-way); negative drops both (axed to
    /// buy). The roster is centred so the panel has no net bias.
    pub axe_steps: f64,
    /// Multiplier on the fleet's base firm size — this counterparty's appetite at
    /// the touch. This is the dominant term in how much of a given order it fills.
    pub size_multiple: f64,
    /// How many price levels this counterparty shows **behind** its top-of-book
    /// (`0` ⇒ firm at the touch only, nothing behind). Together with
    /// [`depth_decay`](Self::depth_decay) this is the quoted depth an aggressive
    /// order walks.
    pub depth_levels: u8,
    /// Each successive depth level's size as a fraction of the one in front of it
    /// (in `(0, 1]`). A platform decays fast (thin behind the touch); a principal
    /// dealer decays slowly (real balance-sheet depth).
    pub depth_decay: f64,
    /// The price concession, in multiples of this counterparty's own half-spread,
    /// between successive depth levels.
    pub depth_step_half_spreads: f64,
    /// Round-trip response latency in microseconds — how far a fresh observation is
    /// back-dated, so a slow counterparty is staleness-decayed by the consolidator
    /// exactly as a slow real feed is.
    pub response_latency_micros: i64,
    /// Quote-refresh cadence in milliseconds: how often this counterparty resamples
    /// its own view of the market. A fast refresher tracks the market level more
    /// closely between emissions.
    pub refresh_millis: i64,
    /// Self-reported feed quality in `[0, 1]`, carried onto every quote and folded
    /// into the composite's confidence.
    pub quality: f64,
    /// An odd constant salting this counterparty's price path off the run seed, so
    /// two members never share a draw even at the same run seed.
    pub seed_salt: u64,
}

impl SimLpProfile {
    /// Whether this profile's id is correctly marked as simulated.
    #[must_use]
    pub fn is_marked_simulated(&self) -> bool {
        self.id.ends_with(SIM_SUFFIX) && self.id.len() > SIM_SUFFIX.len()
    }

    /// This counterparty's effective half-spread given the fleet base, with a small
    /// seeded jitter so successive runs at different seeds are not pixel-identical
    /// while the *ordering* of the roster's characters is preserved.
    ///
    /// The amplitude is [`SPREAD_JITTER`], which is deliberately smaller than half
    /// the narrowest gap between two adjacent profiles' spread multiples: a jitter
    /// wide enough to reorder them would make "who is the tight one" a property of
    /// the run seed rather than of the counterparty, which is exactly the
    /// indistinguishable-fleet problem this roster exists to remove.
    /// [`tests::the_spread_ordering_survives_every_seed`] pins it.
    #[must_use]
    pub fn half_spread(&self, base_half_spread: f64, run_seed: u64) -> f64 {
        base_half_spread
            * self.spread_multiple
            * self.jitter(run_seed, JITTER_SPREAD, SPREAD_JITTER)
    }

    /// This counterparty's effective firm size at the touch, jittered by
    /// [`SIZE_JITTER`] and snapped to a round 100k so quoted quantities read like
    /// real clips.
    #[must_use]
    pub fn firm_size(&self, base_size: f64, run_seed: u64) -> f64 {
        let raw = base_size * self.size_multiple * self.jitter(run_seed, JITTER_SIZE, SIZE_JITTER);
        ((raw / SIZE_GRANULARITY).round() * SIZE_GRANULARITY).max(SIZE_GRANULARITY)
    }

    /// This counterparty's directional axe in mid units, given the fleet's per-step
    /// skew budget. Not jittered: the axe *is* the character.
    #[must_use]
    pub fn axe(&self, skew_step: f64) -> f64 {
        self.axe_steps * skew_step
    }

    /// The refresh cadence in nanoseconds (floored at 1 ms so the sampler advances).
    #[must_use]
    pub fn refresh_nanos(&self) -> i64 {
        self.refresh_millis.max(1).saturating_mul(1_000_000)
    }

    /// The response latency in nanoseconds.
    #[must_use]
    pub fn latency_nanos(&self) -> i64 {
        self.response_latency_micros.max(0).saturating_mul(1_000)
    }

    /// A bounded multiplicative jitter in `[1 − amplitude, 1 + amplitude)`, drawn
    /// deterministically from `(run_seed, this profile's salt, channel)`.
    fn jitter(&self, run_seed: u64, channel: u64, amplitude: f64) -> f64 {
        let u = unit01(run_seed ^ self.seed_salt, channel);
        1.0 + amplitude * (2.0 * u - 1.0)
    }
}

/// The multiplicative jitter amplitude on a counterparty's half-spread. Smaller than
/// half the narrowest adjacent gap in [`OTC_ROSTER`]'s spread multiples (0.07), so it
/// can never reorder the roster's characters.
pub const SPREAD_JITTER: f64 = 0.03;
/// The multiplicative jitter amplitude on a counterparty's firm size. Sizes may
/// legitimately reorder run to run — appetite is the volatile part of a dealer's
/// character, unlike the width it quotes.
pub const SIZE_JITTER: f64 = 0.15;

/// Independent jitter channels (arbitrary distinct odd constants).
const JITTER_SPREAD: u64 = 0x0000_0000_0000_5F03;
const JITTER_SIZE: u64 = 0x0000_0000_0000_7A19;
/// Quoted clips are snapped to this granularity so sizes read like real quantities.
const SIZE_GRANULARITY: f64 = 100_000.0;

/// The **over-the-counter liquidity roster**: the four named simulated
/// counterparties that make the cash / swap / STIR markets the LP simulator streams.
///
/// The four are deliberately spread across the market-structure spectrum so the
/// consolidated best bid/offer has a *reason* to move between them:
///
/// | counterparty | archetype | spread | axe | appetite | depth | latency |
/// |---|---|---|---|---|---|---|
/// | `marketaccess-sim` | electronic platform | tightest (0.86×) | none | 1.0× | 3 thin levels | fastest (0.9 ms) |
/// | `traderweb-sim` | RFQ platform | 0.95× | mild sell | 1.7× | 1 level | 1.6 ms |
/// | `citigroup-sim` | principal dealer | widest (1.12×) | buy | 2.4× | 4 deep levels | slowest (3.4 ms) |
/// | `jpm-sim` | principal dealer | 1.02× | sell | 1.9× | 3 levels | 2.5 ms |
///
/// The axes sum to zero (`0 + 0.45 − 0.75 + 0.30 = 0`), so the panel carries no net
/// directional bias — a trader hitting the composite is not systematically dealt a
/// skewed market, but *which* counterparty wins genuinely depends on the side.
///
/// The **spread band is deliberately narrow** (0.86× to 1.12×). Competing dealers on
/// the same security quote within a fraction of each other, not a factor of two
/// apart; more importantly, a band wider than a member's own per-line private view
/// makes the tightest quoter win the touch on essentially every line at every
/// instant, which turns the street-side LP league table from a measurement into a
/// constant. The four remain unmistakably different — a 30% difference in quoted
/// width is enormous in market terms — and they differ far more sharply on the axes
/// that decide a fill: appetite, depth, latency and cadence.
pub const OTC_ROSTER: &[SimLpProfile] = &[
    SimLpProfile {
        id: "marketaccess-sim",
        display_name: "MarketAccess (simulated)",
        archetype: VenueArchetype::ElectronicPlatform,
        // The tightest market on the panel, but firm for the least — an all-to-all
        // platform wins the touch and gives up size.
        spread_multiple: 0.86,
        axe_steps: 0.0,
        size_multiple: 1.0,
        depth_levels: 3,
        depth_decay: 0.45,
        depth_step_half_spreads: 0.8,
        response_latency_micros: 900,
        refresh_millis: 120,
        quality: 0.98,
        seed_salt: 0x9E37_79B9_7F4A_7C15,
    },
    SimLpProfile {
        id: "traderweb-sim",
        display_name: "TraderWeb (simulated)",
        archetype: VenueArchetype::RequestForQuotePlatform,
        // A platform RFQ answer is the whole clip: large and firm at ONE level,
        // with essentially nothing behind it.
        spread_multiple: 0.95,
        axe_steps: 0.45,
        size_multiple: 1.7,
        depth_levels: 1,
        depth_decay: 0.25,
        depth_step_half_spreads: 1.5,
        response_latency_micros: 1_600,
        refresh_millis: 250,
        quality: 0.95,
        seed_salt: 0xBF58_476D_1CE4_E5B9,
    },
    SimLpProfile {
        id: "citigroup-sim",
        display_name: "Citigroup (simulated)",
        archetype: VenueArchetype::PrincipalDealer,
        // The widest quote and the deepest book: a principal dealer charges for
        // balance sheet and then actually provides it. Axed to buy.
        spread_multiple: 1.12,
        axe_steps: -0.75,
        size_multiple: 2.4,
        depth_levels: 4,
        depth_decay: 0.80,
        depth_step_half_spreads: 0.6,
        response_latency_micros: 3_400,
        refresh_millis: 400,
        quality: 0.91,
        seed_salt: 0x94D0_49BB_1331_11EB,
    },
    SimLpProfile {
        id: "jpm-sim",
        display_name: "JPM (simulated)",
        archetype: VenueArchetype::PrincipalDealer,
        // The all-rounder: mid-market spread, real size, moderate depth. Axed to
        // sell, exactly offsetting the other dealer's buy axe plus the platform's.
        spread_multiple: 1.02,
        axe_steps: 0.30,
        size_multiple: 1.9,
        depth_levels: 3,
        depth_decay: 0.65,
        depth_step_half_spreads: 0.7,
        response_latency_micros: 2_500,
        refresh_millis: 320,
        quality: 0.94,
        seed_salt: 0x2545_F491_4F6C_DD1D,
    },
];

/// The **listed-futures roster**: the single exchange venue the futures simulator
/// presents. A listed contract has one central market, not a panel of competing
/// dealers, so this roster has exactly one member by construction — a second entry
/// would be a second exchange for the same contract, which does not exist.
pub const LISTED_ROSTER: &[SimLpProfile] = &[SimLpProfile {
    id: "cme-sim",
    display_name: "CME (simulated)",
    archetype: VenueArchetype::ListedExchange,
    // Pinned to the contract's own minimum price increment by the futures venue's
    // per-line spread scale; the multiple below is the residual character only.
    spread_multiple: 1.0,
    axe_steps: 0.0,
    // A listed contract's touch is enormous relative to a bond clip, and the book
    // behind it barely decays — that is the defining property of listed liquidity.
    size_multiple: 6.0,
    depth_levels: 5,
    depth_decay: 0.90,
    depth_step_half_spreads: 2.0,
    response_latency_micros: 350,
    refresh_millis: 100,
    quality: 0.99,
    seed_salt: 0x1656_67B1_9C71_1BD3,
}];

/// The profile at panel position `index` of `roster`, or `None` past its end.
///
/// Positions are **not** wrapped. A panel wider than its roster would repeat a
/// connection id, and two members sharing a `VenueId` silently collapse into one
/// contribution inside the consolidator — a fleet that looks like N feeds and
/// consolidates like one. Callers clamp the panel to [`len`] instead.
#[must_use]
pub fn profile_at(roster: &'static [SimLpProfile], index: usize) -> Option<&'static SimLpProfile> {
    roster.get(index)
}

/// Look a profile up by its stable connection id.
#[must_use]
pub fn profile_by_id(roster: &'static [SimLpProfile], id: &str) -> Option<&'static SimLpProfile> {
    roster.iter().find(|p| p.id == id)
}

/// The connection ids of a roster, in panel order.
#[must_use]
pub fn ids(roster: &'static [SimLpProfile]) -> Vec<&'static str> {
    roster.iter().map(|p| p.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn all() -> impl Iterator<Item = &'static SimLpProfile> {
        OTC_ROSTER.iter().chain(LISTED_ROSTER.iter())
    }

    /// Every roster id must carry the `-sim` marker. A simulated counterparty id
    /// reaches a blotter, an LP panel and a booked deal's provenance verbatim; an
    /// unmarked one would read there as a real counterparty.
    #[test]
    fn every_id_is_marked_simulated() {
        for p in all() {
            assert!(
                p.is_marked_simulated(),
                "{} is not marked as a simulated counterparty",
                p.id
            );
            assert!(
                p.display_name.contains("simulated"),
                "{}'s display name must say it is simulated",
                p.id
            );
        }
    }

    /// Ids are unique across BOTH rosters. Two members sharing a `VenueId` collapse
    /// into one contribution inside the consolidator, so a duplicate would silently
    /// shrink the panel rather than fail.
    #[test]
    fn ids_are_globally_unique() {
        let ids: HashSet<&str> = all().map(|p| p.id).collect();
        assert_eq!(ids.len(), OTC_ROSTER.len() + LISTED_ROSTER.len());
    }

    /// The four OTC counterparties must be genuinely distinguishable, not four
    /// copies of one curve: no two may share a spread, an appetite or a depth shape.
    #[test]
    fn the_otc_panel_members_are_actually_different() {
        for (i, a) in OTC_ROSTER.iter().enumerate() {
            for b in &OTC_ROSTER[i + 1..] {
                assert!(
                    (a.spread_multiple - b.spread_multiple).abs() > 0.05,
                    "{} and {} quote the same width",
                    a.id,
                    b.id
                );
                assert!(
                    (a.size_multiple - b.size_multiple).abs() > 0.1,
                    "{} and {} show the same appetite",
                    a.id,
                    b.id
                );
                assert!(
                    a.depth_levels != b.depth_levels
                        || (a.depth_decay - b.depth_decay).abs() > 0.05,
                    "{} and {} show the same depth shape",
                    a.id,
                    b.id
                );
                assert!(
                    a.refresh_millis != b.refresh_millis,
                    "{} and {} refresh on the same cadence",
                    a.id,
                    b.id
                );
                assert!(
                    a.response_latency_micros != b.response_latency_micros,
                    "{} and {} respond with the same latency",
                    a.id,
                    b.id
                );
            }
        }
    }

    /// The panel carries no NET directional bias: a taker hitting the composite must
    /// not be systematically dealt a skewed market, even though which counterparty
    /// wins genuinely depends on the side.
    #[test]
    fn the_otc_axes_are_centred() {
        let net: f64 = OTC_ROSTER.iter().map(|p| p.axe_steps).sum();
        assert!(net.abs() < 1e-12, "panel axe is not centred: {net}");
    }

    /// A profile is a set of constants: the same `(profile, base, seed)` always
    /// yields the same derived parameters, and the jitter stays inside its stated
    /// band so it can never reorder the roster's characters.
    #[test]
    fn derived_parameters_are_reproducible_and_bounded() {
        let base_spread = 2.0e-2;
        let base_size = 1_000_000.0;
        for seed in [0u64, 0x1234_5678, 0xDEAD_BEEF_CAFE_F00D] {
            for p in all() {
                let a = p.half_spread(base_spread, seed);
                let b = p.half_spread(base_spread, seed);
                assert_eq!(a, b, "{} half-spread is not reproducible", p.id);

                let nominal = base_spread * p.spread_multiple;
                assert!(
                    (a - nominal).abs() <= SPREAD_JITTER * nominal + 1e-15,
                    "{}: half-spread {a} escaped the ±{SPREAD_JITTER} jitter band around {nominal}",
                    p.id
                );

                let s = p.firm_size(base_size, seed);
                assert_eq!(s, p.firm_size(base_size, seed));
                let nominal_size = base_size * p.size_multiple;
                assert!(
                    (s - nominal_size).abs() <= SIZE_JITTER * nominal_size + SIZE_GRANULARITY,
                    "{}: size {s} escaped the ±{SIZE_JITTER} jitter band around {nominal_size}",
                    p.id
                );
                assert!(
                    (s / SIZE_GRANULARITY - (s / SIZE_GRANULARITY).round()).abs() < 1e-9,
                    "{}: size {s} is not a round clip",
                    p.id
                );
            }
        }
    }

    /// The jitter band is narrower than half the narrowest adjacent gap in the
    /// roster's spread multiples, so the spread ORDERING is seed-independent — which
    /// is what makes "who is the tight one" a readable property of the panel rather
    /// than of the run.
    #[test]
    fn the_spread_ordering_survives_every_seed() {
        let base = 2.0e-2;
        let mut nominal: Vec<f64> = OTC_ROSTER.iter().map(|p| p.spread_multiple).collect();
        nominal.sort_by(f64::total_cmp);
        let min_gap = nominal
            .windows(2)
            .map(|w| w[1] - w[0])
            .fold(f64::INFINITY, f64::min);
        assert!(
            2.0 * SPREAD_JITTER * nominal[nominal.len() - 1] < min_gap,
            "the ±{SPREAD_JITTER} spread jitter can bridge the {min_gap} gap between \
             two adjacent profiles and reorder the roster"
        );
        for seed in [0u64, 7, 0x1234_5678, u64::MAX, 0xDEAD_BEEF_CAFE_F00D] {
            let tightest = OTC_ROSTER
                .iter()
                .min_by(|a, b| {
                    a.half_spread(base, seed)
                        .total_cmp(&b.half_spread(base, seed))
                })
                .expect("non-empty roster");
            assert_eq!(
                tightest.id, "marketaccess-sim",
                "the tightest quoter changed at seed {seed:#x}"
            );
        }
    }

    /// Depth is well-formed: a decay in `(0, 1]` and a strictly positive price step,
    /// or the ladder walk in `crate::execution` would produce infinite or improving
    /// liquidity behind the touch.
    #[test]
    fn depth_shapes_are_well_formed() {
        for p in all() {
            assert!(
                p.depth_decay > 0.0 && p.depth_decay <= 1.0,
                "{}: decay {} out of range",
                p.id,
                p.depth_decay
            );
            assert!(
                p.depth_step_half_spreads > 0.0,
                "{}: non-positive depth step",
                p.id
            );
            assert!(p.quality > 0.0 && p.quality <= 1.0, "{}: bad quality", p.id);
            assert!(p.refresh_nanos() >= 1_000_000, "{}: sub-ms refresh", p.id);
            assert!(p.latency_nanos() >= 0, "{}: negative latency", p.id);
        }
    }

    /// The listed roster is a single venue by construction — a second entry would be
    /// a second exchange listing the same contract.
    #[test]
    fn the_listed_roster_is_one_venue() {
        assert_eq!(LISTED_ROSTER.len(), 1);
        assert_eq!(LISTED_ROSTER[0].archetype, VenueArchetype::ListedExchange);
        assert_eq!(ids(LISTED_ROSTER), vec!["cme-sim"]);
    }

    /// Panel positions are not wrapped: asking past the end yields `None` rather
    /// than repeating a connection id (which the consolidator would fold into one).
    #[test]
    fn positions_past_the_roster_are_none_not_wrapped() {
        assert_eq!(
            profile_at(OTC_ROSTER, 0).map(|p| p.id),
            Some("marketaccess-sim")
        );
        assert_eq!(profile_at(OTC_ROSTER, OTC_ROSTER.len()), None);
        assert_eq!(
            profile_by_id(OTC_ROSTER, "jpm-sim").map(|p| p.id),
            Some("jpm-sim")
        );
        assert_eq!(profile_by_id(OTC_ROSTER, "nope-sim"), None);
    }
}
