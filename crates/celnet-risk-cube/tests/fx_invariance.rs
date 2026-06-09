//! FX byte/1e-12 non-regression + the `firm_aggregate == single-node` invariant
//! (W5-A §5). These are the binding ADR-0008 guards: the cross-asset generalization
//! moves **no FX ULP**, and the additive roll-up stays associative once non-FX legs
//! are mixed in.

use celnet_core::carry::CarryInputs;
use celnet_core::is_close;
use celnet_risk_cube::{
    BookId, Cube, DeskId, DimensionId, EntityId, FactKey, FactMeasure, LocationId, PositionId,
    RiskFact, Scenario, TraderId, VegaPillar, VegaPillarMap,
};
use celnet_risk_normalize::{
    AssetPricer, CanonicalLeaf, PositionRisk, StaticSpotResolver, canonicalize,
};
use celnet_types::{
    Carry, Ccy, CcyPair, DeltaConvention, EquityRef, Metal, MetalPair, OptionType, PremiumStyle,
    Symbol, Underlying, VanillaInputs,
};

struct OnePillar;
impl VegaPillarMap for OnePillar {
    fn pillar_of(&self, _leaf: &CanonicalLeaf, pos: &PositionRisk) -> VegaPillar {
        VegaPillar::new((pos.inputs.t * 365.0).round() as u32, 5000)
    }
}

fn eurusd() -> CcyPair {
    CcyPair::new(Ccy::EUR, Ccy::USD)
}
fn usdjpy() -> CcyPair {
    CcyPair::new(Ccy::USD, Ccy::JPY)
}

fn fx(pair: CcyPair, opt: OptionType, n: f64, vi: VanillaInputs) -> PositionRisk {
    PositionRisk::fx(
        pair,
        opt,
        n,
        vi,
        DeltaConvention::SpotUnadjusted,
        PremiumStyle::DomesticPips,
    )
}

fn metal(opt: OptionType, n: f64, vi: VanillaInputs) -> PositionRisk {
    let u = Underlying::Metal(MetalPair::new(Metal::Gold, Ccy::USD));
    PositionRisk::carry(
        u.clone(),
        opt,
        n,
        CarryInputs::new(
            vi.spot,
            vi.strike,
            vi.vol,
            vi.t,
            u,
            Carry::FxRates {
                r_dom: vi.r_dom,
                r_for: vi.r_for,
            },
        ),
    )
}

/// An equity position: `(spot, strike, vol, t)` market state + `(r, b)` carry.
fn equity(opt: OptionType, n: f64, mkt: (f64, f64, f64, f64), carry: (f64, f64)) -> PositionRisk {
    let (spot, strike, vol, t) = mkt;
    let (r, b) = carry;
    let u = Underlying::Equity(EquityRef::new(Symbol::new("ACME", ""), Ccy::USD));
    PositionRisk::carry(
        u.clone(),
        opt,
        n,
        CarryInputs::new(spot, strike, vol, t, u, Carry::CostOfCarry { r, b }),
    )
}

fn fact(id: u32, trader: u32, book: u32, desk: u32, position: &PositionRisk) -> RiskFact {
    RiskFact {
        position_id: PositionId(id),
        key: FactKey {
            trader: TraderId(trader),
            book: BookId(book),
            desk: DeskId(desk),
            underlying: position.underlying.clone(),
            location: LocationId(1),
            entity: EntityId(1),
        },
        measure: FactMeasure {
            leaf: canonicalize(position).unwrap(),
            position: position.clone(),
            exotic: None,
        },
        surface_version: 1,
    }
}

/// G11: an EURUSD book built two ways — (a) via the generalized `PositionRisk::fx` →
/// `AssetPricer`/seam, (b) by calling `celnet_vanilla` directly and hand-canonicalizing
/// — has `delta_base` **`to_bits`-identical** (convention-pinned ⇒ bit-exact) and the
/// rest 1e-12. The generalization moves no FX ULP.
#[test]
fn fx_firm_aggregate_byte_identical() {
    let specs = [
        (
            eurusd(),
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        ),
        (
            eurusd(),
            OptionType::Put,
            -5_000_000.0,
            VanillaInputs::new(1.10, 1.08, 0.11, 0.5, 0.04, 0.02),
        ),
        (
            eurusd(),
            OptionType::Call,
            7_000_000.0,
            VanillaInputs::new(1.10, 1.15, 0.09, 1.0, 0.04, 0.02),
        ),
    ];

    // (a) Seam path.
    let mut cube = Cube::new();
    for (i, (pair, opt, n, vi)) in specs.iter().enumerate() {
        cube.upsert(fact(i as u32 + 1, 1, 1, 1, &fx(*pair, *opt, *n, *vi)));
    }
    let firm = cube.firm_aggregate(&OnePillar);

    // (b) Direct celnet-vanilla hand-canonicalization.
    let mut want_delta = 0.0;
    let mut want_vega = 0.0;
    let mut want_gamma = 0.0;
    let mut want_premium = 0.0;
    for (pair, opt, n, vi) in &specs {
        let _ = pair;
        want_delta +=
            celnet_vanilla::convention_delta(DeltaConvention::SpotUnadjusted, *opt, vi) * n;
        let g = celnet_vanilla::greeks(*opt, vi);
        want_vega += g.vega * n;
        want_gamma += g.gamma * n;
        want_premium += g.price * n;
    }

    // delta is convention-pinned ⇒ bit-identical.
    assert_eq!(
        firm.net_greeks.delta_base.to_bits(),
        want_delta.to_bits(),
        "FX delta must be to_bits-identical through the generalized seam"
    );
    assert!(is_close(firm.net_greeks.vega, want_vega, 1e-12, 1e-6));
    assert!(is_close(firm.net_greeks.gamma, want_gamma, 1e-12, 1e-9));
    assert!(is_close(
        firm.net_greeks.premium_quote,
        want_premium,
        1e-12,
        1e-6
    ));
}

/// G12: a firm aggregate over N mixed-asset facts (distinct trader/book/desk keys)
/// equals the aggregate of a **single node** holding the same N positions, to 1e-12 —
/// every `net_greeks` field, the vega ladder total, the numeraire-view delta, and the
/// non-additive `node_var_es` oracle.
#[test]
fn firm_aggregate_equals_single_node() {
    let positions = [
        fx(
            eurusd(),
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        ),
        fx(
            usdjpy(),
            OptionType::Put,
            8_000_000.0,
            VanillaInputs::new(156.0, 154.0, 0.11, 0.5, 0.01, 0.05),
        ),
        metal(
            OptionType::Put,
            -3_000.0,
            VanillaInputs::new(2_000.0, 1_950.0, 0.15, 1.0, 0.05, 0.01),
        ),
        equity(
            OptionType::Call,
            1_000.0,
            (100.0, 105.0, 0.20, 1.0),
            (0.03, 0.01),
        ),
    ];

    // Firm aggregate across distinct org keys.
    let mut cube = Cube::new();
    for (i, p) in positions.iter().enumerate() {
        let k = i as u32 + 1;
        cube.upsert(fact(k, k, k, k, p));
    }
    let firm = cube.firm_aggregate(&OnePillar);

    // "Single node" reference: one cube with all positions under ONE org key.
    let mut single = Cube::new();
    for (i, p) in positions.iter().enumerate() {
        single.upsert(fact(i as u32 + 1, 1, 1, 1, p));
    }
    let node = single.firm_aggregate(&OnePillar);

    assert!(is_close(
        firm.net_greeks.delta_base,
        node.net_greeks.delta_base,
        1e-12,
        1e-6
    ));
    assert!(is_close(
        firm.net_greeks.vega,
        node.net_greeks.vega,
        1e-12,
        1e-6
    ));
    assert!(is_close(
        firm.net_greeks.gamma,
        node.net_greeks.gamma,
        1e-12,
        1e-9
    ));
    assert!(is_close(
        firm.net_greeks.theta,
        node.net_greeks.theta,
        1e-12,
        1e-6
    ));
    assert!(is_close(
        firm.net_greeks.premium_quote,
        node.net_greeks.premium_quote,
        1e-12,
        1e-6
    ));
    assert!(is_close(
        firm.vega_ladder.total(),
        node.vega_ladder.total(),
        1e-12,
        1e-6
    ));

    // Numeraire view (USD): every fiat leg nets identically; metals/equity net their
    // USD funding leg, their asset-unit base leg is not currency-converted (the honest
    // fiat-only collapse). Both views see the same legs ⇒ equal.
    // The metal's base leg nets in XAU (its as_ccy_pair projection is XAU/USD), so the
    // numeraire collapse needs an XAU→USD rate (the gold spot, 2000). The equity's base
    // leg is an asset unit with no CcyPair projection, so only its USD funding leg is in
    // the Ccy vector (the honest fiat-only collapse) — no equity rate needed.
    let usd = StaticSpotResolver::new(
        Ccy::USD,
        &[
            (Ccy::EUR, 1.10),
            (Ccy::JPY, 1.0 / 156.0),
            (Ccy::XAU, 2_000.0),
        ],
    );
    let fv = firm.numeraire_view(&usd).unwrap();
    let nv = node.numeraire_view(&usd).unwrap();
    assert!(is_close(fv.delta_numeraire, nv.delta_numeraire, 1e-9, 1e-3));

    // Non-additive guard: node VaR/ES (bump-revalue oracle through the seam) agrees.
    let scen: Vec<Scenario> = (-10..=10)
        .filter(|i| *i != 0)
        .map(|i| Scenario::spot(f64::from(i) * 0.004))
        .collect();
    let firm_ve = Cube::node_var_es(&AssetPricer, &firm, &scen, 0.99);
    let node_ve = Cube::node_var_es(&AssetPricer, &node, &scen, 0.99);
    assert!(is_close(firm_ve.var, node_ve.var, 1e-12, 1e-6));
    assert!(is_close(firm_ve.es, node_ve.es, 1e-12, 1e-6));
}

/// G13: `firm == Σ group_by(Desk)` to 1e-12 on a **mixed-asset** book — introducing
/// the non-FX legs did not break the associative additive contract.
#[test]
fn rollup_associative_across_assets() {
    let positions = [
        (
            1u32,
            fx(
                eurusd(),
                OptionType::Call,
                10_000_000.0,
                VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            ),
        ),
        (
            2,
            metal(
                OptionType::Put,
                -3_000.0,
                VanillaInputs::new(2_000.0, 1_950.0, 0.15, 1.0, 0.05, 0.01),
            ),
        ),
        (
            1,
            equity(
                OptionType::Call,
                1_000.0,
                (100.0, 105.0, 0.20, 1.0),
                (0.03, 0.01),
            ),
        ),
        (
            2,
            fx(
                usdjpy(),
                OptionType::Put,
                8_000_000.0,
                VanillaInputs::new(156.0, 154.0, 0.11, 0.5, 0.01, 0.05),
            ),
        ),
    ];
    let mut cube = Cube::new();
    for (i, (desk, p)) in positions.iter().enumerate() {
        cube.upsert(fact(i as u32 + 1, i as u32 + 1, i as u32 + 1, *desk, p));
    }
    let firm = cube.firm_aggregate(&OnePillar);
    let by_desk = cube.group_by(DimensionId::Desk, &OnePillar);
    assert!(by_desk.len() >= 2, "the mixed book must span ≥2 desks");

    let sum_delta: f64 = by_desk.iter().map(|d| d.net_greeks.delta_base).sum();
    let sum_vega: f64 = by_desk.iter().map(|d| d.net_greeks.vega).sum();
    let sum_premium: f64 = by_desk.iter().map(|d| d.net_greeks.premium_quote).sum();
    assert!(is_close(firm.net_greeks.delta_base, sum_delta, 1e-12, 1e-6));
    assert!(is_close(firm.net_greeks.vega, sum_vega, 1e-12, 1e-6));
    assert!(is_close(
        firm.net_greeks.premium_quote,
        sum_premium,
        1e-12,
        1e-6
    ));
}
