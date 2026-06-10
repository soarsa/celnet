//! Additive measures (`docs/RISK-HIERARCHY.md` §2.5).
//!
//! The first-order and second-order Greeks of a *canonical* leaf are **additive**:
//! they aggregate by associative/commutative summation, so a node total is the
//! plain sum of its constituents' canonical Greeks, and a roll-up is incremental
//! (a single new fact touches O(depth) ancestor sums). This is only sound because
//! the leaves are already convention-free (`celnet-risk-normalize` §2.2) — adding
//! raw broker Greeks across conventions would be meaningless.
//!
//! Two additive measures are carried:
//!
//! - **[`NetGreeks`]** — the summed canonical Greek set (delta-base, gamma, vega,
//!   theta, vanna, volga, charm, speed, zomma, color), each a notional-scaled sum.
//! - **[`VegaLadder`]** — vega bucketed by `(tenor × delta)` pillars, the same
//!   `BucketedRisk` shape the engine already produces (`docs/RISK-HIERARCHY.md`
//!   §2.3). Buckets sum element-wise.
//!
//! Both are pure value types with an associative `+`/`add_leaf`, so the cube can
//! either fold them across a node's facts or maintain them incrementally.

use celnet_risk_normalize::CanonicalLeaf;

/// The summed canonical Greek set for a set of leaves — the additive first/second
/// order roll-up (`docs/RISK-HIERARCHY.md` §2.5).
///
/// Every field is a sum of the constituents' notional-scaled canonical Greeks.
/// `delta_base` is the netted **base-currency** delta amount; it is *not* yet
/// numeraire-converted (that is `celnet-risk-normalize`'s `Numeraire`, applied at
/// presentation time, because the base currency differs per pair). Summing
/// `delta_base` across pairs is therefore only meaningful per-pair; for a
/// cross-pair node use [`crate::cube::Cube::numeraire_view`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NetGreeks {
    /// Σ spot-unadjusted base-currency delta amount.
    pub delta_base: f64,
    /// Σ gamma.
    pub gamma: f64,
    /// Σ vega (per 1.0 absolute vol).
    pub vega: f64,
    /// Σ theta (per year).
    pub theta: f64,
    /// Σ vanna.
    pub vanna: f64,
    /// Σ volga.
    pub volga: f64,
    /// Σ charm.
    pub charm: f64,
    /// Σ speed.
    pub speed: f64,
    /// Σ zomma.
    pub zomma: f64,
    /// Σ color.
    pub color: f64,
    /// Σ premium (quote-currency PV line, carried separately from delta).
    pub premium_quote: f64,
}

impl NetGreeks {
    /// The empty (all-zero) accumulator.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            delta_base: 0.0,
            gamma: 0.0,
            vega: 0.0,
            theta: 0.0,
            vanna: 0.0,
            volga: 0.0,
            charm: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
            premium_quote: 0.0,
        }
    }

    /// Accumulate one canonical leaf's Greeks into this total.
    pub fn add_leaf(&mut self, leaf: &CanonicalLeaf) {
        let g = &leaf.greeks;
        self.delta_base += g.delta_base;
        self.gamma += g.gamma;
        self.vega += g.vega;
        self.theta += g.theta;
        self.vanna += g.vanna;
        self.volga += g.volga;
        self.charm += g.charm;
        self.speed += g.speed;
        self.zomma += g.zomma;
        self.color += g.color;
        self.premium_quote += leaf.premium_quote;
    }
}

impl core::ops::Add for NetGreeks {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self {
            delta_base: self.delta_base + o.delta_base,
            gamma: self.gamma + o.gamma,
            vega: self.vega + o.vega,
            theta: self.theta + o.theta,
            vanna: self.vanna + o.vanna,
            volga: self.volga + o.volga,
            charm: self.charm + o.charm,
            speed: self.speed + o.speed,
            zomma: self.zomma + o.zomma,
            color: self.color + o.color,
            premium_quote: self.premium_quote + o.premium_quote,
        }
    }
}

/// A `(tenor × delta)` vega pillar coordinate — the bucket key for [`VegaLadder`].
///
/// Tenor and delta are quantized to integer pillars so buckets are exactly
/// comparable across leaves (floating tenors/deltas would never net). The
/// quantization is the caller's responsibility — the cube maps a leaf's
/// `(time, moneyness)` onto the desired regulatory or internal pillar grid before
/// bucketing. `tenor_days` is the pillar's calendar-day label; `delta_bp` is the
/// signed delta pillar in basis points of delta (e.g. 2500 = 0.25Δ).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VegaPillar {
    /// The tenor pillar, labelled in calendar days.
    pub tenor_days: u32,
    /// The signed delta pillar in basis-points of delta (0.25Δ → 2500).
    pub delta_bp: i32,
}

impl VegaPillar {
    /// Construct a pillar from a calendar-day tenor and a signed delta in
    /// basis-points-of-delta.
    #[must_use]
    pub const fn new(tenor_days: u32, delta_bp: i32) -> Self {
        Self {
            tenor_days,
            delta_bp,
        }
    }
}

/// Vega bucketed by `(tenor × delta)` pillar (`docs/RISK-HIERARCHY.md` §2.3) — an
/// additive measure: buckets sum element-wise across leaves.
///
/// Backed by a small association vector (the live pillar set in a book is bounded
/// — the regulatory FX-vega vertices are ≤ a dozen tenors × a handful of deltas),
/// so accumulation is allocation-light and the ladder is `Clone`. Vega is in each
/// leaf's premium currency; cross-pair *vega* netting must first numeraire-convert
/// through the premium ccy (§2.2/§2.3 coupling, handled by `celnet-risk-normalize`'s
/// `Numeraire`) — the ladder itself only sums vegas that already share a pillar
/// *and* a premium currency, so the caller buckets per `(pair, pillar)` when a
/// cross-currency ladder is needed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VegaLadder {
    buckets: Vec<(VegaPillar, f64)>,
}

impl VegaLadder {
    /// An empty ladder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `vega` into the `(tenor × delta)` pillar bucket.
    pub fn add(&mut self, pillar: VegaPillar, vega: f64) {
        if let Some(slot) = self.buckets.iter_mut().find(|(p, _)| *p == pillar) {
            slot.1 += vega;
        } else {
            self.buckets.push((pillar, vega));
        }
    }

    /// Merge another ladder into this one (element-wise pillar sum).
    pub fn merge(&mut self, other: &VegaLadder) {
        for &(p, v) in &other.buckets {
            self.add(p, v);
        }
    }

    /// The vega in a single pillar (`0.0` if the pillar is absent).
    #[must_use]
    pub fn vega_in(&self, pillar: VegaPillar) -> f64 {
        self.buckets
            .iter()
            .find(|(p, _)| *p == pillar)
            .map_or(0.0, |(_, v)| *v)
    }

    /// The non-empty pillar buckets, in insertion order.
    pub fn pillars(&self) -> impl Iterator<Item = (VegaPillar, f64)> + '_ {
        self.buckets.iter().copied()
    }

    /// The total vega summed over all pillars (only meaningful when all pillars
    /// share a premium currency — see the type-level note).
    #[must_use]
    pub fn total(&self) -> f64 {
        self.buckets.iter().map(|(_, v)| *v).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_risk_normalize::CanonicalGreeks;
    use celnet_types::{Ccy, CcyPair, Underlying};

    /// A leaf whose 11 measure lines carry DISTINCT dyadic values scaled by `m`
    /// (so every per-field sum below is float-exact and a mutation of any single
    /// field's accumulation is caught by that field's bit assert).
    fn leaf(m: f64) -> CanonicalLeaf {
        CanonicalLeaf {
            underlying: Underlying::Fx(CcyPair::new(Ccy::EUR, Ccy::USD)),
            spot: 1.25,
            greeks: CanonicalGreeks {
                delta_base: 1.0 * m,
                gamma: 2.0 * m,
                vega: 4.0 * m,
                theta: 8.0 * m,
                vanna: 16.0 * m,
                volga: 32.0 * m,
                charm: 64.0 * m,
                speed: 128.0 * m,
                zomma: 256.0 * m,
                color: 512.0 * m,
            },
            premium_quote: 1024.0 * m,
            vega_premium_ccy: Ccy::USD,
            quoted_was_premium_adjusted: false,
        }
    }

    fn fields(n: &NetGreeks) -> [f64; 11] {
        [
            n.delta_base,
            n.gamma,
            n.vega,
            n.theta,
            n.vanna,
            n.volga,
            n.charm,
            n.speed,
            n.zomma,
            n.color,
            n.premium_quote,
        ]
    }

    /// `zero()` is exactly all-zero, `add_leaf` accumulates EVERY field by `+`
    /// (bit-exact dyadic sums), and the `Add` operator agrees field-for-field with
    /// the fold.
    #[test]
    fn net_greeks_accumulate_every_field_exactly() {
        assert_eq!(fields(&NetGreeks::zero()), [0.0; 11]);

        let mut acc = NetGreeks::zero();
        acc.add_leaf(&leaf(1.0));
        acc.add_leaf(&leaf(0.25));
        // Hand sums: base weights × (1 + 0.25) = ×1.25 exactly.
        let want: [f64; 11] = [
            1.25, 2.5, 5.0, 10.0, 20.0, 40.0, 80.0, 160.0, 320.0, 640.0, 1280.0,
        ];
        for (i, (g, w)) in fields(&acc).iter().zip(want).enumerate() {
            assert_eq!(g.to_bits(), w.to_bits(), "add_leaf field #{i}");
        }

        // The Add operator: a + b == the same fold, every field bit-equal.
        let mut a = NetGreeks::zero();
        a.add_leaf(&leaf(1.0));
        let mut b = NetGreeks::zero();
        b.add_leaf(&leaf(0.25));
        let sum = a + b;
        for (i, (g, w)) in fields(&sum).iter().zip(want).enumerate() {
            assert_eq!(g.to_bits(), w.to_bits(), "Add field #{i}");
        }
        // A negative leaf genuinely subtracts (catches an `abs`-style mutation).
        let mut c = acc;
        c.add_leaf(&leaf(-0.25));
        let back: [f64; 11] = [
            1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0, 512.0, 1024.0,
        ];
        for (i, (g, w)) in fields(&c).iter().zip(back).enumerate() {
            assert_eq!(g.to_bits(), w.to_bits(), "signed field #{i}");
        }
    }

    /// The ladder nets same-pillar adds, keeps distinct pillars apart (tenor OR
    /// delta differing), merges element-wise, reports absent pillars as exactly
    /// 0.0, iterates in insertion order, and totals by plain sum.
    #[test]
    fn vega_ladder_buckets_merge_and_total_exactly() {
        let p1y = VegaPillar::new(365, 5000);
        let p6m = VegaPillar::new(183, 5000);
        let p1y25d = VegaPillar::new(365, 2500); // same tenor, different delta

        let mut l = VegaLadder::new();
        l.add(p1y, 2.0);
        l.add(p6m, 8.0);
        l.add(p1y, 0.5); // nets into p1y
        l.add(p1y25d, 16.0); // distinct bucket despite equal tenor
        assert_eq!(l.vega_in(p1y).to_bits(), 2.5_f64.to_bits());
        assert_eq!(l.vega_in(p6m).to_bits(), 8.0_f64.to_bits());
        assert_eq!(l.vega_in(p1y25d).to_bits(), 16.0_f64.to_bits());
        // Absent pillar reads exactly 0.0.
        assert_eq!(
            l.vega_in(VegaPillar::new(30, 5000)).to_bits(),
            0.0_f64.to_bits()
        );
        assert_eq!(l.total().to_bits(), 26.5_f64.to_bits());
        // Insertion order is the documented iteration order.
        let order: Vec<VegaPillar> = l.pillars().map(|(p, _)| p).collect();
        assert_eq!(order, vec![p1y, p6m, p1y25d]);

        // Merge: overlapping pillar nets, new pillar appends.
        let mut other = VegaLadder::new();
        other.add(p6m, -4.0);
        other.add(VegaPillar::new(30, 5000), 1.0);
        l.merge(&other);
        assert_eq!(l.vega_in(p6m).to_bits(), 4.0_f64.to_bits());
        assert_eq!(
            l.vega_in(VegaPillar::new(30, 5000)).to_bits(),
            1.0_f64.to_bits()
        );
        assert_eq!(l.total().to_bits(), 23.5_f64.to_bits());
        assert_eq!(l.pillars().count(), 4);

        // The pillar constructor stores its coordinates as given.
        let p = VegaPillar::new(91, -2500);
        assert_eq!(p.tenor_days, 91);
        assert_eq!(p.delta_bp, -2500);
    }
}
