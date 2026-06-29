//! Parity row: the **risk-cube exotic leg** prices/Greeks/curvature/PnL through the
//! injected [`ExoticLegPricer`] seam exactly match the closed-form `celnet-exotics`
//! engines (Reiner-Rubinstein single barrier, European digital).
//!
//! These bodies were moved verbatim out of `celnet-risk-cube/src/exotic.rs`'s
//! in-crate `#[cfg(test)] mod tests` when arch-program item E inverted the cube's
//! dependency on `celnet-exotics`: the cube no longer depends on the pricing crate,
//! so a test that needs the *concrete* barrier/digital closed form (the independent
//! oracle these tests compare against) cannot live in the cube. `celnet-parity`
//! already depends on `celnet-exotics`, so the same [`ExoticEngine`] the **server**
//! injects backs the cube reducers here, and the oracle calls
//! (`single_barrier_price` / `digital_price` / `digital_greeks`) stay exactly as
//! before. Every assertion is preserved byte-identically; only the `ExoticEngine`
//! seam argument was threaded through the cube's `ExoticLeg` methods + free
//! functions (the signature change arch-program item E introduced).
//!
//! The pure-helper test of the cube's private FD bump (`spot_bump` / `with_spot`)
//! stays in-crate — it needs no pricer.

use celnet_core::ExoticLegPricer;
use celnet_exotics::{
    BarrierKind, BarrierStyle, ExoticInputs, SingleBarrier, digital_greeks, digital_price,
    single_barrier_price,
};
use celnet_risk_cube::{ExoticKind, ExoticLeg, Scenario, exotic_curvature_legs, exotic_node_pnl};
use celnet_types::{Ccy, CcyPair, DigitalKind, OptionType, VanillaInputs};

use celnet_core::is_close;

/// The concrete exotic-leg pricer over the `celnet-exotics` closed forms — the
/// SAME shape the server injects (`celnet-server/.../risk/exotic_pricer.rs`): the
/// identical `VanillaInputs → ExoticInputs` projection (`From<&VanillaInputs>`,
/// byte-identical FX two-rate accessors) and the identical
/// `single_barrier_price` / `digital_price` / `digital_greeks` calls, so the cube
/// reprices the real exotic byte-for-byte.
#[derive(Debug, Clone, Copy, Default)]
struct ExoticEngine;

impl ExoticLegPricer for ExoticEngine {
    #[inline]
    fn unit_price(&self, kind: ExoticKind, inputs: &VanillaInputs) -> f64 {
        let i: ExoticInputs = inputs.into();
        match kind {
            ExoticKind::SingleBarrier(spec) => single_barrier_price(&i, spec),
            ExoticKind::Digital(k) => digital_price(k, &i),
        }
    }

    #[inline]
    fn digital_greeks(&self, kind: DigitalKind, inputs: &VanillaInputs) -> (f64, f64, f64) {
        let dg = digital_greeks(kind, &inputs.into());
        (dg.delta, dg.gamma, dg.vega)
    }
}

fn eurusd() -> CcyPair {
    CcyPair::new(Ccy::EUR, Ccy::USD)
}

fn up_out_call() -> SingleBarrier {
    SingleBarrier {
        kind: BarrierKind {
            up: true,
            style: BarrierStyle::KnockOut,
            option: OptionType::Call,
        },
        strike: 1.10,
        barrier: 1.25,
        rebate: 0.0,
    }
}

/// The FX projection of the cube's `apply_fx` (a crate-private helper inverted from
/// the asset-agnostic carry shock packing): spot relative, vol absolute, and the FX
/// rates from `Δr_dom = discount_abs`, `Δr_for = discount_abs − carry_abs`.
/// Replicated here (code-disjoint from the cube) so the moved tests keep their exact
/// independent reprice oracle.
fn apply_fx(scenario: Scenario, i: &VanillaInputs) -> VanillaInputs {
    VanillaInputs::new(
        i.spot * (1.0 + scenario.spot_rel),
        i.strike,
        i.vol + scenario.vol_abs,
        i.t,
        i.r_dom + scenario.discount_abs,
        i.r_for + (scenario.discount_abs - scenario.carry_abs),
    )
}

/// The leaf's premium line equals the closed-form barrier price × notional.
#[test]
fn barrier_leaf_premium_matches_closed_form() {
    let inputs = VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02);
    let leg = ExoticLeg::new(
        eurusd(),
        ExoticKind::SingleBarrier(up_out_call()),
        10_000_000.0,
        inputs,
    );
    let leaf = leg.canonical_leaf(&ExoticEngine);
    let want = single_barrier_price(&(&inputs).into(), up_out_call()) * 10_000_000.0;
    assert!(is_close(leaf.premium_quote, want, 1e-12, 1e-6));
}

/// A digital leg's leaf delta/gamma/vega equal the closed-form digital Greeks ×
/// notional (the published exact values, not FD).
#[test]
fn digital_leaf_uses_closed_form_greeks() {
    let kind = DigitalKind::cash(OptionType::Call);
    let inputs = VanillaInputs::new(1.30, 1.32, 0.12, 0.75, 0.03, 0.01);
    let n = 5_000_000.0;
    let leg = ExoticLeg::new(
        CcyPair::new(Ccy::GBP, Ccy::USD),
        ExoticKind::Digital(kind),
        n,
        inputs,
    );
    let leaf = leg.canonical_leaf(&ExoticEngine);
    let dg = digital_greeks(kind, &(&inputs).into());
    assert!(is_close(leaf.greeks.delta_base, dg.delta * n, 1e-12, 1e-3));
    assert!(is_close(leaf.greeks.gamma, dg.gamma * n, 1e-12, 1e-3));
    assert!(is_close(leaf.greeks.vega, dg.vega * n, 1e-12, 1e-3));
}

/// The exotic P&L is the real exotic reprice difference (not a vanilla proxy):
/// shocking spot up toward the knock-out barrier of an up-and-out call DESTROYS
/// value (the option approaches extinction), so the long leg's P&L is negative —
/// the opposite sign a long *vanilla* call would show for the same up-shock.
#[test]
fn knock_out_pnl_is_exotic_not_vanilla() {
    let inputs = VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02);
    let leg = ExoticLeg::new(
        eurusd(),
        ExoticKind::SingleBarrier(up_out_call()),
        10_000_000.0,
        inputs,
    );
    // A +5% spot move pushes toward the 1.25 up-and-out barrier.
    let up = Scenario::spot(0.05);
    let exotic_pnl = leg.pnl(&ExoticEngine, up);
    // Independent: real barrier reprice difference (FX projection of the shock).
    let shocked = apply_fx(up, &inputs);
    let want = (single_barrier_price(&(&shocked).into(), up_out_call())
        - single_barrier_price(&(&inputs).into(), up_out_call()))
        * 10_000_000.0;
    assert!(is_close(exotic_pnl, want, 1e-9, 1e-3));
    // A long up-and-out call LOSES value as spot rises toward the barrier.
    assert!(
        exotic_pnl < 0.0,
        "up-and-out call should lose value on an up-move toward the barrier, got {exotic_pnl}"
    );
    // Sanity: a long vanilla call would GAIN on the same up-move — proving the
    // exotic path is genuinely different from a vanilla proxy.
    let vanilla_up = celnet_vanilla::price(OptionType::Call, &shocked)
        - celnet_vanilla::price(OptionType::Call, &inputs);
    assert!(vanilla_up > 0.0);
}

/// **Rate shocks map back to the FX two-rate basis** (the `apply_fx` inverse
/// packing): an exotic leg's P&L under a combined spot/vol/discount/carry
/// scenario equals the closed-form reprice at `(spot·(1+s), vol+v,
/// r_dom+Δr_dom, r_for+Δr_for)` with the rates re-derived BY HAND from the
/// carry shocks (`Δr_dom = discount_abs`, `Δr_for = discount_abs − carry_abs`).
#[test]
fn exotic_rate_shock_maps_to_fx_two_rate_basis() {
    let inputs = VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02);
    let n = 10_000_000.0;
    let leg = ExoticLeg::new(
        eurusd(),
        ExoticKind::SingleBarrier(up_out_call()),
        n,
        inputs,
    );
    // Dyadic shocks; built via the FX-rates constructor so the hand mapping
    // below is the published two-rate contract, not the carry packing.
    let (s, v, dr_dom, dr_for) = (0.03125, 0.0078125, 0.015625, -0.001953125);
    let sc = Scenario::fx_rates(s, v, dr_dom, dr_for);
    let shocked = VanillaInputs::new(
        inputs.spot * (1.0 + s),
        inputs.strike,
        inputs.vol + v,
        inputs.t,
        inputs.r_dom + dr_dom,
        inputs.r_for + dr_for,
    );
    let want = (single_barrier_price(&(&shocked).into(), up_out_call())
        - single_barrier_price(&(&inputs).into(), up_out_call()))
        * n;
    assert!(is_close(leg.pnl(&ExoticEngine, sc), want, 1e-12, 1e-6));
    // The rate legs genuinely matter (vacuity guard): zeroing them changes P&L.
    let spot_vol_only = Scenario::fx_rates(s, v, 0.0, 0.0);
    assert!((leg.pnl(&ExoticEngine, sc) - leg.pnl(&ExoticEngine, spot_vol_only)).abs() > 1.0);

    // base_value and quote_ccy bookkeeping.
    assert!(is_close(
        leg.base_value(&ExoticEngine),
        single_barrier_price(&(&inputs).into(), up_out_call()) * n,
        1e-15,
        1e-9
    ));
    assert_eq!(leg.quote_ccy(), Ccy::USD);
}

/// **Curvature legs = two revaluations net of the leg's own FD delta**,
/// re-derived longhand from the closed form (the MAR21 CVR± arithmetic), and
/// `exotic_curvature_legs` sums leg pairs element-wise.
///
/// The barrier sits at 1.50 so the +15% shock (1.10 → 1.265) keeps the option
/// ALIVE with a materially nonzero up price — a mutant that corrupts the
/// up-shocked spot (e.g. `spot·(1.0·rw)`) then moves the up leg by orders of
/// magnitude instead of comparing knocked-out ≈ 0 against deep-OTM ≈ 0 (the
/// gap the first mutation run exposed). A digital leg is pinned the same way:
/// its value moves in BOTH shock directions.
#[test]
fn exotic_curvature_legs_match_independent_reprice() {
    let inputs = VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02);
    let n = 10_000_000.0;
    let rw = 0.15;
    let spec = SingleBarrier {
        kind: BarrierKind {
            up: true,
            style: BarrierStyle::KnockOut,
            option: OptionType::Call,
        },
        strike: 1.10,
        barrier: 1.50, // alive at the +15% shocked spot 1.265
        rebate: 0.0,
    };
    let leg = ExoticLeg::new(eurusd(), ExoticKind::SingleBarrier(spec), n, inputs);

    let pr = |spot: f64| {
        single_barrier_price(
            &(&VanillaInputs::new(
                spot,
                inputs.strike,
                inputs.vol,
                inputs.t,
                inputs.r_dom,
                inputs.r_for,
            ))
                .into(),
            spec,
        )
    };
    let base = pr(inputs.spot) * n;
    let up = pr(inputs.spot * (1.0 + rw)) * n;
    let down = pr(inputs.spot * (1.0 - rw)) * n;
    assert!(up > 1e-3 * n, "the up-shocked barrier price must be alive");
    // The leg's linear term uses its own central-FD delta; re-derive it with
    // the same documented bump (relative 1e-4, absolute floor 1e-7).
    let h = (inputs.spot.abs() * 1e-4).max(1e-7);
    let fd_delta = (pr(inputs.spot + h) - pr(inputs.spot - h)) / (2.0 * h);
    let linear = fd_delta * n * rw * inputs.spot;
    let want_up = -((up - base) - linear);
    let want_down = -((down - base) + linear);

    let (got_up, got_down) = leg.curvature_legs(&ExoticEngine, rw);
    assert!(
        is_close(got_up, want_up, 1e-9, 1e-3),
        "CVR+ {got_up} vs {want_up}"
    );
    assert!(is_close(got_down, want_down, 1e-9, 1e-3));

    // The digital's curvature legs, pinned the same longhand way (its price
    // moves in both directions, so each shocked-spot expression is pinned).
    let dk = DigitalKind::cash(OptionType::Put);
    let d_inputs = VanillaInputs::new(1.10, 1.09, 0.11, 0.5, 0.03, 0.01);
    let dn = -4_000_000.0;
    let dleg = ExoticLeg::new(eurusd(), ExoticKind::Digital(dk), dn, d_inputs);
    let dpr = |spot: f64| {
        digital_price(
            dk,
            &(&VanillaInputs::new(
                spot,
                d_inputs.strike,
                d_inputs.vol,
                d_inputs.t,
                d_inputs.r_dom,
                d_inputs.r_for,
            ))
                .into(),
        )
    };
    let d_base = dpr(d_inputs.spot) * dn;
    let d_up = dpr(d_inputs.spot * (1.0 + rw)) * dn;
    let d_down = dpr(d_inputs.spot * (1.0 - rw)) * dn;
    let dh = (d_inputs.spot.abs() * 1e-4).max(1e-7);
    let d_fd = (dpr(d_inputs.spot + dh) - dpr(d_inputs.spot - dh)) / (2.0 * dh);
    let d_linear = d_fd * dn * rw * d_inputs.spot;
    let d_want_up = -((d_up - d_base) - d_linear);
    let d_want_down = -((d_down - d_base) + d_linear);
    let (d_got_up, d_got_down) = dleg.curvature_legs(&ExoticEngine, rw);
    assert!(
        is_close(d_got_up, d_want_up, 1e-9, 1e-3),
        "digital CVR+ {d_got_up} vs {d_want_up}"
    );
    assert!(is_close(d_got_down, d_want_down, 1e-9, 1e-3));
    // Vacuity guards: both shocked digital values genuinely differ from base.
    assert!((d_up - d_base).abs() > 1.0 && (d_down - d_base).abs() > 1.0);

    // The summed pair is element-wise across legs.
    let leg2 = ExoticLeg::new(
        eurusd(),
        ExoticKind::Digital(DigitalKind::cash(OptionType::Put)),
        -4_000_000.0,
        VanillaInputs::new(1.10, 1.09, 0.11, 0.5, 0.03, 0.01),
    );
    let (u1, d1) = leg.curvature_legs(&ExoticEngine, rw);
    let (u2, d2) = leg2.curvature_legs(&ExoticEngine, rw);
    let (su, sd) = exotic_curvature_legs(&ExoticEngine, &[leg, leg2], rw);
    assert!(is_close(su, u1 + u2, 1e-12, 1e-9));
    assert!(is_close(sd, d1 + d2, 1e-12, 1e-9));
    // And the empty set is exactly (0, 0).
    let (zu, zd) = exotic_curvature_legs(&ExoticEngine, &[], rw);
    assert_eq!(zu.to_bits(), 0.0_f64.to_bits());
    assert_eq!(zd.to_bits(), 0.0_f64.to_bits());
}

/// **Independent analytic oracle for the FULL FD Greek set**: a barrier so far
/// from spot it can never knock (≈15σ) prices as the vanilla, so every
/// higher-order FD Greek on the leaf must match `celnet_vanilla::greeks`'
/// closed-form set — a fully code-disjoint oracle for theta / vanna / volga /
/// charm / speed / zomma / color (a stencil-coefficient or sign mutant moves
/// these by orders of magnitude, far beyond the FD truncation tolerance).
#[test]
fn far_barrier_leaf_greeks_match_vanilla_analytic() {
    let inputs = VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02);
    let n = 5_000_000.0;
    let far = SingleBarrier {
        kind: BarrierKind {
            up: true,
            style: BarrierStyle::KnockOut,
            option: OptionType::Call,
        },
        strike: 1.12,
        barrier: 5.0, // ln(5/1.1)/(σ√t) ≈ 15σ: knock probability ≈ 0
        rebate: 0.0,
    };
    let leg = ExoticLeg::new(eurusd(), ExoticKind::SingleBarrier(far), n, inputs);
    let leaf = leg.canonical_leaf(&ExoticEngine);
    let g = celnet_vanilla::greeks(OptionType::Call, &inputs);
    let checks = [
        ("delta", leaf.greeks.delta_base, g.delta_spot * n),
        ("gamma", leaf.greeks.gamma, g.gamma * n),
        ("vega", leaf.greeks.vega, g.vega * n),
        ("theta", leaf.greeks.theta, g.theta * n),
        ("vanna", leaf.greeks.vanna, g.vanna * n),
        ("volga", leaf.greeks.volga, g.volga * n),
        ("charm", leaf.greeks.charm, g.charm * n),
        ("speed", leaf.greeks.speed, g.speed * n),
        ("zomma", leaf.greeks.zomma, g.zomma * n),
        ("color", leaf.greeks.color, g.color * n),
    ];
    for (name, got, want) in checks {
        assert!(
            is_close(got, want, 5e-4, 1e-9 * n),
            "{name}: FD {got} vs analytic {want}"
        );
        assert!(want.abs() > 0.0, "{name} oracle must not be vacuous");
    }
    assert!(is_close(leaf.premium_quote, g.price * n, 1e-9, 1e-9 * n));
    // Leaf bookkeeping: the exotic leaf carries its pair/spot/premium-ccy and
    // is never premium-adjusted.
    assert_eq!(leaf.underlying, celnet_types::Underlying::Fx(eurusd()));
    assert_eq!(leaf.spot.to_bits(), inputs.spot.to_bits());
    assert_eq!(leaf.vega_premium_ccy, Ccy::USD);
    assert!(!leaf.quoted_was_premium_adjusted);
}

/// The FD time bump stays strictly inside the tenor: `canonical_greeks` caps
/// `dt` at `t/2`, so even an ultra-short-dated leg (here `t = 4e-7`y ≈ 12.6 s,
/// below the `1e-6` absolute floor, so the `t/2` cap BINDS at `dt = 2e-7`)
/// reprices every `t − dt` leg at a strictly positive time. Breaking the cap
/// arithmetic (`t·0.5` → `t + 0.5` or `t / 0.5`) drives `t − dt` negative and
/// `√t` poisons the time-direction Greeks (theta/charm/color) with NaN —
/// pinned by finiteness of the FULL Greek set plus the far-barrier
/// vanilla-limit analytic oracle on the two non-degenerate time Greeks of a
/// saturated deep-ITM call (at `d₁ ≈ 3.3e3`, `N(d₁) = 1` and `n(d₁) = 0` to
/// double precision, so per unit `θ → ±(r_f·S·e^{−r_f t} − r_d·K·e^{−r_d t})`
/// ≈ ∓0.0188 and charm → `−r_f·e^{−r_f t}` ≈ −0.02 — both nonzero).
#[test]
fn ultra_short_tenor_time_bump_stays_inside_the_tenor() {
    let inputs = VanillaInputs::new(1.30, 1.12, 0.10, 4e-7, 0.04, 0.02);
    let n = 5_000_000.0;
    let far = SingleBarrier {
        kind: BarrierKind {
            up: true,
            style: BarrierStyle::KnockOut,
            option: OptionType::Call,
        },
        strike: 1.12,
        barrier: 5.0, // knock probability ≈ 0 ⇒ the vanilla limit
        rebate: 0.0,
    };
    let leg = ExoticLeg::new(eurusd(), ExoticKind::SingleBarrier(far), n, inputs);
    let g = leg.canonical_leaf(&ExoticEngine).greeks;
    // Kill core: a broken dt cap reprices at t − dt < 0, and the closed
    // forms' √t turns theta/charm/color into NaN. Every Greek must be finite.
    for (name, v) in [
        ("delta", g.delta_base),
        ("gamma", g.gamma),
        ("vega", g.vega),
        ("theta", g.theta),
        ("vanna", g.vanna),
        ("volga", g.volga),
        ("charm", g.charm),
        ("speed", g.speed),
        ("zomma", g.zomma),
        ("color", g.color),
    ] {
        assert!(v.is_finite(), "{name} must be finite, got {v}");
    }
    let a = celnet_vanilla::greeks(OptionType::Call, &inputs);
    assert!(a.theta.abs() > 1e-3, "theta oracle must not be vacuous");
    assert!(a.charm.abs() > 1e-3, "charm oracle must not be vacuous");
    // Theta: the FD signal `V(t+dt) − V(t−dt)` ≈ 7.5e-9 against ~1e-16 of
    // f64 rounding in the ~0.18-scale prices ⇒ expected FD error ~1e-8 rel.
    assert!(
        is_close(g.theta, a.theta * n, 5e-4, 1e-9 * n),
        "theta: FD {} vs analytic {}",
        g.theta,
        a.theta * n
    );
    // Charm carries the documented cross-difference cancellation budget: the
    // outer signal is ~2.1e-12 against ~2e-16 absolute rounding noise in the
    // ~2.6e-4-scale inner spot differences ⇒ ~1e-4 expected relative error;
    // 1e-3 keeps an order of magnitude of margin and can never pass a NaN.
    assert!(
        is_close(g.charm, a.charm * n, 1e-3, 1e-9 * n),
        "charm: FD {} vs analytic {}",
        g.charm,
        a.charm * n
    );
    // Delta saturates at e^{−r_f·t}·N(d₁) ≈ 1 — pinned so the leg provably
    // sits in the vanilla limit the time-Greek oracle above relies on.
    assert!(is_close(g.delta_base, a.delta_spot * n, 5e-4, 1e-9 * n));
}

/// `exotic_node_pnl` is the plain sum of the legs' P&L (two-leg pin + empty
/// set exactly zero), and a digital leg's value line is the closed-form digital
/// price × notional.
#[test]
fn exotic_node_pnl_sums_legs_exactly() {
    let l1 = ExoticLeg::new(
        eurusd(),
        ExoticKind::SingleBarrier(up_out_call()),
        10_000_000.0,
        VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02),
    );
    let kind = DigitalKind::cash(OptionType::Call);
    let l2 = ExoticLeg::new(
        CcyPair::new(Ccy::GBP, Ccy::USD),
        ExoticKind::Digital(kind),
        -5_000_000.0,
        VanillaInputs::new(1.30, 1.32, 0.12, 0.75, 0.03, 0.01),
    );
    let sc = Scenario::spot(0.02);
    let want = l1.pnl(&ExoticEngine, sc) + l2.pnl(&ExoticEngine, sc);
    assert_eq!(
        exotic_node_pnl(&ExoticEngine, &[l1, l2], sc).to_bits(),
        want.to_bits()
    );
    // Empty set: numerically zero (the iterator-sum identity is −0.0 on this
    // toolchain; the zero's sign is not contractual).
    assert_eq!(exotic_node_pnl(&ExoticEngine, &[], sc), 0.0);
    // Digital value line == closed form × notional (the unit_price digital arm).
    let vi = VanillaInputs::new(1.30, 1.32, 0.12, 0.75, 0.03, 0.01);
    assert!(is_close(
        l2.base_value(&ExoticEngine),
        digital_price(kind, &(&vi).into()) * -5_000_000.0,
        1e-15,
        1e-9
    ));
}

/// FD Greeks of a barrier leg match an independent in-test central FD of the
/// closed-form price (the canonical leaf does not silently zero higher orders).
#[test]
fn barrier_fd_greeks_match_independent_fd() {
    let inputs = VanillaInputs::new(1.10, 1.08, 0.11, 0.5, 0.03, 0.01);
    let spec = up_out_call();
    let n = 7_000_000.0;
    let leg = ExoticLeg::new(eurusd(), ExoticKind::SingleBarrier(spec), n, inputs);
    let leaf = leg.canonical_leaf(&ExoticEngine);

    // Independent central FD of the closed-form price for delta/gamma/vega.
    let pr = |s: f64, vol: f64| {
        single_barrier_price(
            &(&VanillaInputs::new(s, inputs.strike, vol, inputs.t, inputs.r_dom, inputs.r_for))
                .into(),
            spec,
        )
    };
    let ds = (inputs.spot * 1e-4).max(1e-7);
    let dv = 1e-4;
    let s = inputs.spot;
    let v = inputs.vol;
    let base = pr(s, v);
    let ref_delta = (pr(s + ds, v) - pr(s - ds, v)) / (2.0 * ds) * n;
    let ref_gamma = (pr(s + ds, v) - 2.0 * base + pr(s - ds, v)) / (ds * ds) * n;
    let ref_vega = (pr(s, v + dv) - pr(s, v - dv)) / (2.0 * dv) * n;
    assert!(is_close(leaf.greeks.delta_base, ref_delta, 1e-9, 1e-2));
    assert!(is_close(leaf.greeks.gamma, ref_gamma, 1e-9, 1e-2));
    assert!(is_close(leaf.greeks.vega, ref_vega, 1e-9, 1e-2));
}
