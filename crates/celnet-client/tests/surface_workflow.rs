//! Trader-workflow integration tests for the surface + risk SDK.
//!
//! * **Scenario 3 — a quant marks the surface from broker ATM/RR/BF, then reads
//!   the smile across deltas and the arbitrage report.** Driven through the typed
//!   [`celnet_client::Client::mark_surface`] / [`celnet_client::Client::get_smile`]:
//!   the calibrated 50Δ pillar reproduces the marked ATM vol, the wings carry the
//!   marked skew, and the typed [`celnet_client::ArbReport`] reports
//!   butterfly-arbitrage-freedom — exactly what the server's own surface tests
//!   assert, but through the SDK's typed [`celnet_client::Smile`].
//! * **Scenario 4 — a risk manager runs a spot/vol shock grid.** Driven through
//!   [`celnet_client::Client::scenario`]: every node's repriced price equals a
//!   first-principles `celnet-vanilla` price at the node's shocked market, and the
//!   unshocked node equals the base price.
//!
//! Every body is hard wall-clock bounded and every network await is bounded.

mod common;

use std::time::Duration;

use celnet_client::{BrokerQuoteSet, InstrumentSpec, Quantity, ShockAxis, ShockFactor, Side};
use celnet_core::is_close;
use celnet_types::{OptionType, VanillaInputs};

use common::{
    STEP_DEADLINE, TEST_DEADLINE, conventions, eurusd, live_market, start_edge_and_client,
    vanilla_call,
};

/// Scenario 3: a quant marks the surface from a broker ATM/RR/BF quote set and
/// reads back a typed calibrated smile with an arbitrage report. The 50Δ pillar
/// reproduces the marked ATM vol and the calibrated smile is butterfly-arb-free.
#[tokio::test]
async fn quant_marks_surface_reads_smile_and_arb_report() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;

        let broker = BrokerQuoteSet::three_point(1.0, 0.105, -0.0040, 0.0020);
        let marked = tokio::time::timeout(
            STEP_DEADLINE,
            client.mark_surface(eurusd(), &[broker], conventions()),
        )
        .await
        .expect("mark_surface returns in time")
        .expect("mark_surface succeeds");

        assert!(marked.surface_version >= 1, "a surface version is stamped");
        assert_eq!(marked.smiles.len(), 1, "one smile per broker quote set");
        let smile = &marked.smiles[0];
        assert!(is_close(smile.tenor_years, 1.0, 1e-12, 1e-12));
        assert_eq!(smile.points.len(), 5, "delta-axis pillars reported");

        // The arbitrage report is typed and a quant branches on it directly.
        assert!(
            smile.arbitrage.butterfly_arbitrage_free,
            "a mild EURUSD smile is butterfly-arbitrage-free"
        );

        // The 50Δ (ATM) pillar reproduces the marked ATM vol, via the typed helper.
        let atm = smile.atm_vol().expect("a 50Δ pillar exists");
        assert!(
            is_close(atm, broker.atm_vol, 5e-3, 5e-3),
            "ATM pillar vol {atm} ~ marked {}",
            broker.atm_vol
        );

        // The skew is present: with a negative 25Δ RR the 25Δ put wing vol exceeds
        // the 25Δ call wing vol (puts richer than calls).
        let call_25 = smile.vol_at_delta(0.25).expect("25Δ call pillar");
        let put_25 = smile.vol_at_delta(-0.25).expect("25Δ put pillar");
        assert!(
            put_25 > call_25,
            "negative RR ⇒ 25Δ put vol {put_25} > 25Δ call vol {call_25}"
        );

        // A subsequent GetSmile returns a calibrated, sane delta-axis slice.
        let read = tokio::time::timeout(
            STEP_DEADLINE,
            client.get_smile(eurusd(), 1.0, conventions()),
        )
        .await
        .expect("get_smile returns in time")
        .expect("get_smile succeeds");
        assert_eq!(read.points.len(), 5);
        for p in &read.points {
            assert!(p.vol > 0.0 && p.vol < 1.0, "vol {} in range", p.vol);
        }

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// eSSVI client-parity proof: a quant marks the surface under the **extended
/// surface** calibration family (the eSSVI / maturity-dependent-ρ family carried
/// on the wire as `SMILE_MODEL_EXTENDED_SURFACE=4`) through the typed
/// [`celnet_client::Client::mark_surface_with`] over a REAL edge, and reads back
/// calibrated smiles whose arbitrage-note provenance reflects the extended family
/// (`model=extended-surface`).
///
/// This mirrors the existing market-hedge mark flow but selects the extended
/// surface via the ergonomic [`celnet_client::Calibration::ExtendedSurface`]
/// variant — the SDK already maps it to wire tag 4 via `calibration_to_wire`
/// (`Calibration = celnet_types::SmileModel`, which carries `ExtendedSurface`), so
/// this proves the SDK leg of the eSSVI client-parity contract end-to-end: the
/// model is selectable, the smile calibrates, and the family the server actually
/// used is visible to the consumer through the arb-note provenance channel.
#[tokio::test]
async fn quant_marks_surface_under_extended_surface_family() {
    use celnet_client::Calibration;

    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;

        let broker = BrokerQuoteSet::three_point(1.0, 0.105, -0.0040, 0.0020);

        // Baseline market-hedge mark, for a same-input differential comparison: the
        // arb-note provenance must name a DIFFERENT family than the extended mark.
        let baseline = tokio::time::timeout(
            STEP_DEADLINE,
            client.mark_surface(eurusd(), &[broker], conventions()),
        )
        .await
        .expect("baseline mark_surface returns in time")
        .expect("baseline mark_surface succeeds");
        let baseline_note = baseline.smiles[0].arbitrage.note.clone();
        assert!(
            baseline_note.contains("model=market-hedge"),
            "baseline note names the market-hedge family, got {baseline_note:?}"
        );

        // Mark the SAME broker set under the extended-surface (eSSVI) family via the
        // ergonomic typed selector — the SDK encodes it to wire tag 4.
        let marked = tokio::time::timeout(
            STEP_DEADLINE,
            client.mark_surface_with(
                eurusd(),
                &[broker],
                conventions(),
                Calibration::ExtendedSurface,
            ),
        )
        .await
        .expect("extended mark_surface_with returns in time")
        .expect("extended mark_surface_with succeeds");

        assert!(
            marked.surface_version >= 1,
            "the extended mark stamps a surface version"
        );
        assert_eq!(marked.smiles.len(), 1, "one smile per broker quote set");
        let smile = &marked.smiles[0];
        assert!(is_close(smile.tenor_years, 1.0, 1e-12, 1e-12));
        assert_eq!(smile.points.len(), 5, "delta-axis pillars reported");

        // The smile is genuinely calibrated under the extended family: every
        // delta-axis pillar carries a sane positive vol.
        for p in &smile.points {
            assert!(
                p.vol > 0.0 && p.vol < 1.0,
                "extended-surface pillar vol {} in range",
                p.vol
            );
        }
        // The calibrated extended smile is butterfly-arbitrage-free for this mild
        // EURUSD quote set.
        assert!(
            smile.arbitrage.butterfly_arbitrage_free,
            "the extended-surface mark is butterfly-arbitrage-free"
        );
        // The 50Δ pillar reproduces the marked ATM vol.
        let atm = smile.atm_vol().expect("a 50Δ pillar exists");
        assert!(
            is_close(atm, broker.atm_vol, 5e-3, 5e-3),
            "extended ATM pillar vol {atm} ~ marked {}",
            broker.atm_vol
        );

        // THE eSSVI CLIENT-PARITY ASSERTION (typed provenance, W12): the smile's
        // TYPED `arbitrage.model` field is the authoritative calibration family the
        // server marked under — read directly, NOT by regex over the note. It must be
        // the extended-surface family, and distinct from the baseline market-hedge.
        assert_eq!(
            smile.arbitrage.model,
            Calibration::ExtendedSurface,
            "the typed provenance field names the extended-surface (eSSVI) family"
        );
        assert_eq!(
            baseline.smiles[0].arbitrage.model,
            Calibration::MarketHedge,
            "the baseline mark's typed provenance is the market-hedge family"
        );
        assert_ne!(
            smile.arbitrage.model, baseline.smiles[0].arbitrage.model,
            "the typed provenance distinguishes the two families of the same inputs"
        );
        // The human-readable note still embeds the `model=` token (kept for eyes),
        // but it is no longer the authoritative channel — the typed field above is.
        let note = smile.arbitrage.note.clone();
        assert!(
            note.contains("model=extended-surface"),
            "the human-readable note still names the family, got {note:?}"
        );
        assert_ne!(
            note, baseline_note,
            "the human note also differs from the market-hedge mark of the same inputs"
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Scenario 4: a risk manager runs a spot × vol shock grid. Every node's repriced
/// price equals a first-principles `celnet-vanilla` price at the node's shocked
/// market, and the unshocked `[0, 0]` node equals the base price.
#[tokio::test]
async fn risk_manager_runs_spot_vol_shock_grid() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;

        let strike = 1.12;
        let base = live_market();
        let spot_steps = vec![-0.10, 0.0, 0.10];
        let vol_steps = vec![0.0, 0.50];
        let axes = vec![
            ShockAxis::relative(ShockFactor::Spot, spot_steps.clone()),
            ShockAxis::relative(ShockFactor::Vol, vol_steps.clone()),
        ];

        let grid = tokio::time::timeout(
            STEP_DEADLINE,
            client.scenario(&vanilla_call(strike), base, &axes, conventions()),
        )
        .await
        .expect("scenario returns in time")
        .expect("scenario succeeds");

        assert_eq!(
            grid.nodes.len(),
            spot_steps.len() * vol_steps.len(),
            "the grid is the Cartesian product of the axes"
        );

        // Every node reprices exactly against a direct GK price at its shocked
        // (spot, vol) — the SDK surfaces the maker's deterministic value verbatim.
        for node in &grid.nodes {
            let shocked = node.shocked_market;
            let direct = celnet_vanilla::price(
                OptionType::Call,
                &VanillaInputs::new(
                    shocked.spot,
                    strike,
                    shocked.vol,
                    1.0,
                    shocked.r_dom,
                    shocked.r_for,
                ),
            );
            assert!(
                is_close(node.greeks.price, direct, 1e-10, 1e-10),
                "node price {} != direct {direct} at spot {} vol {}",
                node.greeks.price,
                shocked.spot,
                shocked.vol
            );
        }

        // The unshocked node equals the base price, via the typed lookup helper.
        let base_node = grid
            .node_with_shocks(&[0.0, 0.0])
            .expect("the [0,0] node exists");
        let base_direct = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(base.spot, strike, base.vol, 1.0, base.r_dom, base.r_for),
        );
        assert!(is_close(base_node.greeks.price, base_direct, 1e-12, 1e-12));

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// One-shot pricing through the SDK equals a direct GK price at the supplied
/// market context — the `Client::price` path surfaces the maker's deterministic
/// computation verbatim.
#[tokio::test]
async fn one_shot_price_equals_direct() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;

        let strike = 1.08;
        let market = live_market();
        let priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&vanilla_call(strike), market, conventions()),
        )
        .await
        .expect("price returns in time")
        .expect("price succeeds");

        let direct = celnet_vanilla::greeks(
            OptionType::Call,
            &VanillaInputs::new(
                market.spot,
                strike,
                market.vol,
                1.0,
                market.r_dom,
                market.r_for,
            ),
        );
        assert!(is_close(priced.greeks.price, direct.price, 1e-12, 1e-12));
        assert!(is_close(priced.greeks.vega, direct.vega, 1e-12, 1e-12));
        assert!(is_close(priced.greeks.gamma, direct.gamma, 1e-12, 1e-12));
        assert!(is_close(priced.resolved_strike, strike, 1e-12, 1e-12));

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Wave-1 products through the SDK: a variance swap, a volatility swap, and an
/// arithmetic Asian price end-to-end via `Client::price` and equal the
/// `celnet-exotics` closed forms (the independent oracle) at the live market —
/// the api-first parity proof that the new wire products are reachable from the
/// SDK exactly like the existing ones.
#[tokio::test]
async fn wave1_swaps_and_asian_price_through_sdk() {
    use celnet_client::{AsianMethod, AsianTerms, AveragingStyle};
    use celnet_exotics::{
        AnalyticAsian, VarSwapContext, curran_price, fair_variance, fair_volatility,
    };

    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;
        let market = live_market();
        let template = VanillaInputs::new(
            market.spot,
            market.spot,
            market.vol,
            1.0,
            market.r_dom,
            market.r_for,
        );
        let ctx = VarSwapContext::from_inputs(&template);
        let flat = celnet_core::FlatSmile::new(market.vol);

        // Variance swap: headline price == fair variance strike K_var; vol == √K_var.
        let var_spec = InstrumentSpec::variance_swap(
            eurusd(),
            celnet_types::Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            0.0,
        );
        let var_priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&var_spec, market, conventions()),
        )
        .await
        .expect("var swap price returns")
        .expect("var swap price succeeds");
        let var_oracle = fair_variance(&flat, &ctx);
        assert!(
            is_close(
                var_priced.greeks.price,
                var_oracle.fair_variance,
                1e-9,
                1e-12
            ),
            "SDK K_var {} != oracle {}",
            var_priced.greeks.price,
            var_oracle.fair_variance
        );
        // The SDK echoes the fair variance strike as the resolved strike.
        assert!(is_close(
            var_priced.resolved_strike,
            var_oracle.fair_variance,
            1e-9,
            1e-12
        ));

        // Volatility swap: headline price == fair vol strike K_vol.
        let vol_spec = InstrumentSpec::volatility_swap(
            eurusd(),
            celnet_types::Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            0.0,
        );
        let vol_priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&vol_spec, market, conventions()),
        )
        .await
        .expect("vol swap price returns")
        .expect("vol swap price succeeds");
        let vol_oracle = fair_volatility(&flat, &ctx);
        assert!(
            is_close(vol_priced.greeks.price, vol_oracle.fair_vol, 1e-9, 1e-12),
            "SDK K_vol {} != oracle {}",
            vol_priced.greeks.price,
            vol_oracle.fair_vol
        );

        // Arithmetic Asian (Curran, 12 discrete fixings): SDK price == oracle.
        let asian_strike = 1.10;
        let asian_spec = InstrumentSpec::asian_option(
            eurusd(),
            celnet_types::Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            AsianTerms::fresh_discrete(OptionType::Call, asian_strike, 12)
                .method(AsianMethod::Curran),
        );
        // Exercise the fluent continuous variant too (compile + value parity).
        assert_eq!(
            AsianTerms::fresh_continuous(OptionType::Put, 1.0).averaging,
            AveragingStyle::Continuous
        );
        let asian_priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&asian_spec, market, conventions()),
        )
        .await
        .expect("asian price returns")
        .expect("asian price succeeds");
        let asian_inputs = VanillaInputs::new(
            market.spot,
            asian_strike,
            market.vol,
            1.0,
            market.r_dom,
            market.r_for,
        );
        let asian_oracle = curran_price(
            &asian_inputs,
            AnalyticAsian::fresh_discrete(OptionType::Call, asian_strike, 12),
        );
        assert!(
            is_close(asian_priced.greeks.price, asian_oracle, 1e-9, 1e-12),
            "SDK Asian {} != oracle {}",
            asian_priced.greeks.price,
            asian_oracle
        );
        assert!(asian_priced.greeks.price > 0.0 && asian_priced.greeks.vega > 0.0);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Wave-2 products through the SDK: a forward-start vanilla, a plain cliquet, a
/// clamped (capped) cliquet that carries an honest standard error, and a quanto
/// vanilla/digital — each priced end-to-end via `Client::price` and equal to the
/// `celnet-exotics` closed forms / MC (the independent oracle) at the live
/// market. The api-first parity proof for the W2 wire products.
#[tokio::test]
async fn wave2_forward_start_cliquet_quanto_price_through_sdk() {
    use celnet_client::{CliquetTerms, ForwardStartTerms, QuantoPayoff, QuantoTerms};
    use celnet_exotics::{
        Cliquet, CliquetMcConfig, CliquetSchedule, ForwardStart, QuantoParams,
        cliquet_price_capped_mc, cliquet_price_plain, forward_start_price, quanto_digital_price,
        quanto_vanilla_price,
    };

    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;
        let market = live_market();
        let inputs = VanillaInputs::new(
            market.spot,
            market.spot,
            market.vol,
            1.0,
            market.r_dom,
            market.r_for,
        );

        // Forward-start vanilla (reset at 3m, ATM-forward reset).
        let fs_spec = InstrumentSpec::forward_start(
            eurusd(),
            celnet_types::Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            ForwardStartTerms::new(OptionType::Call, 1.0, 0.25),
        );
        let fs_priced =
            tokio::time::timeout(STEP_DEADLINE, client.price(&fs_spec, market, conventions()))
                .await
                .expect("forward-start price returns")
                .expect("forward-start price succeeds");
        let fs_oracle = forward_start_price(
            &inputs,
            ForwardStart {
                option: OptionType::Call,
                moneyness: 1.0,
                reset: 0.25,
                expiry: 1.0,
            },
        );
        assert!(
            is_close(fs_priced.greeks.price, fs_oracle, 1e-9, 1e-12),
            "SDK forward-start {} != oracle {}",
            fs_priced.greeks.price,
            fs_oracle
        );
        assert!(fs_priced.price_std_error.is_none());

        // Plain (unclamped) cliquet == Σ forward-start legs (closed form).
        let periods = 4u32;
        let plain_spec = InstrumentSpec::cliquet(
            eurusd(),
            celnet_types::Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            CliquetTerms::plain(OptionType::Call, 1.0, periods),
        );
        let plain_priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&plain_spec, market, conventions()),
        )
        .await
        .expect("plain cliquet price returns")
        .expect("plain cliquet price succeeds");
        let plain_oracle = cliquet_price_plain(
            &inputs,
            &Cliquet {
                option: OptionType::Call,
                moneyness: 1.0,
                schedule: CliquetSchedule::equal(periods as usize, 1.0),
                local_floor: None,
                local_cap: None,
                global_floor: None,
                global_cap: None,
            },
        );
        assert!(
            is_close(plain_priced.greeks.price, plain_oracle, 1e-10, 1e-12),
            "SDK plain cliquet {} != oracle {}",
            plain_priced.greeks.price,
            plain_oracle
        );
        assert!(plain_priced.price_std_error.is_none());

        // Capped cliquet: MC price == the celnet-exotics MC with the SAME seed,
        // and the SDK surfaces the standard error honestly.
        let cap = 0.03;
        let pairs = 20_000u32;
        let seed = 0xC0FFEE_u64;
        let capped_spec = InstrumentSpec::cliquet(
            eurusd(),
            celnet_types::Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            CliquetTerms::plain(OptionType::Call, 1.0, periods)
                .local(Some(0.0), Some(cap))
                .monte_carlo(pairs, seed),
        );
        let capped_priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&capped_spec, market, conventions()),
        )
        .await
        .expect("capped cliquet price returns")
        .expect("capped cliquet price succeeds");
        let capped_oracle = cliquet_price_capped_mc(
            &inputs,
            &Cliquet {
                option: OptionType::Call,
                moneyness: 1.0,
                schedule: CliquetSchedule::equal(periods as usize, 1.0),
                local_floor: Some(0.0),
                local_cap: Some(cap),
                global_floor: None,
                global_cap: None,
            },
            CliquetMcConfig {
                pairs: pairs as usize,
                seed,
            },
        );
        assert!(
            is_close(
                capped_priced.greeks.price,
                capped_oracle.price,
                1e-12,
                1e-12
            ),
            "SDK capped cliquet {} != oracle MC {}",
            capped_priced.greeks.price,
            capped_oracle.price
        );
        let stderr = capped_priced
            .price_std_error
            .expect("clamped cliquet must surface std-error through the SDK");
        assert!(
            is_close(stderr, capped_oracle.std_error, 1e-12, 1e-12) && stderr > 0.0,
            "SDK std-error {} != oracle {}",
            stderr,
            capped_oracle.std_error
        );

        // MC-honesty across the QUOTE path (the path the GUI live-WS and Excel use,
        // not just the one-shot price): a clamped-cliquet `Quote` MUST carry
        // `price_std_error`, and a closed-form product's quote MUST NOT. This gates
        // the wire fix that the Quote message + WS quote codec carry the std-error.
        let capped_quote = tokio::time::timeout(
            STEP_DEADLINE,
            client
                .request_quote(capped_spec.clone(), conventions())
                .request(),
        )
        .await
        .expect("capped cliquet quote returns")
        .expect("capped cliquet quote succeeds");
        let quote_stderr = capped_quote
            .price_std_error
            .expect("clamped cliquet QUOTE must surface std-error (WS/SDK quote path)");
        assert!(quote_stderr > 0.0, "quote std-error must be positive");
        let plain_quote = tokio::time::timeout(
            STEP_DEADLINE,
            client
                .request_quote(plain_spec.clone(), conventions())
                .request(),
        )
        .await
        .expect("plain cliquet quote returns")
        .expect("plain cliquet quote succeeds");
        assert!(
            plain_quote.price_std_error.is_none(),
            "a closed-form (plain cliquet) quote must NOT carry a std-error"
        );

        // Quanto vanilla and digital.
        let quanto_strike = 1.10;
        let qv_spec = InstrumentSpec::quanto(
            eurusd(),
            celnet_types::Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            QuantoTerms::new(
                QuantoPayoff::Vanilla,
                OptionType::Call,
                quanto_strike,
                0.09,
                -0.3,
            ),
        );
        let qv_priced =
            tokio::time::timeout(STEP_DEADLINE, client.price(&qv_spec, market, conventions()))
                .await
                .expect("quanto vanilla price returns")
                .expect("quanto vanilla price succeeds");
        let qv_oracle = quanto_vanilla_price(
            OptionType::Call,
            &VanillaInputs::new(
                market.spot,
                quanto_strike,
                market.vol,
                1.0,
                market.r_dom,
                market.r_for,
            ),
            QuantoParams::new(0.09, -0.3),
        );
        assert!(
            is_close(qv_priced.greeks.price, qv_oracle, 1e-9, 1e-12),
            "SDK quanto vanilla {} != oracle {}",
            qv_priced.greeks.price,
            qv_oracle
        );

        let qd_spec = InstrumentSpec::quanto(
            eurusd(),
            celnet_types::Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            QuantoTerms::new(QuantoPayoff::Digital, OptionType::Put, 1.12, 0.07, 0.4),
        );
        let qd_priced =
            tokio::time::timeout(STEP_DEADLINE, client.price(&qd_spec, market, conventions()))
                .await
                .expect("quanto digital price returns")
                .expect("quanto digital price succeeds");
        let qd_oracle = quanto_digital_price(
            OptionType::Put,
            &VanillaInputs::new(
                market.spot,
                1.12,
                market.vol,
                1.0,
                market.r_dom,
                market.r_for,
            ),
            QuantoParams::new(0.07, 0.4),
        );
        assert!(
            is_close(qd_priced.greeks.price, qd_oracle, 1e-9, 1e-12),
            "SDK quanto digital {} != oracle {}",
            qd_priced.greeks.price,
            qd_oracle
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Wave-3 products through the SDK: a TARF (Monte-Carlo, carrying an honest
/// standard error on BOTH the price and the RFQ quote path), an accumulator
/// (Monte-Carlo, std-error), a continuously-monitored lookback (exact closed
/// form, no std-error), and a discretely-monitored lookback (Monte-Carlo,
/// std-error) — each priced end-to-end via `Client::price` / `Rfq::request` and
/// equal to the `celnet-exotics` reference (the independent oracle) at the live
/// market. The api-first parity + MC-honesty proof for the W3 wire products.
///
/// The MC budgets are deliberately small: the SDK price == the celnet-exotics MC
/// is **bit-reproducible from the shared (seed, pairs, observations)**, so the
/// equality is exact at ANY budget — the count only sets the std-error magnitude
/// (which must stay strictly positive), not the agreement. Small budgets keep the
/// finite-difference Greek strip's repeated repricings inside the test deadline.
#[tokio::test]
async fn wave3_tarf_accumulator_lookback_price_through_sdk() {
    use celnet_client::{
        AccumulatorMonitoring, AccumulatorTerms, LookbackStyle, LookbackTerms, TarfRedemption,
        TarfTerms,
    };
    use celnet_exotics::{
        Accumulator as ExAccumulator, AccumulatorMcConfig, Lookback as ExLookback,
        LookbackMcConfig, LookbackStyle as ExLookbackStyle, Monitoring as ExMonitoring,
        RedemptionStyle as ExRedemptionStyle, Tarf as ExTarf, TarfMcConfig, accumulator_price,
        floating_lookback_price, lookback_mc, tarf_price,
    };

    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;
        let market = live_market();
        let inputs = VanillaInputs::new(
            market.spot,
            market.spot,
            market.vol,
            1.0,
            market.r_dom,
            market.r_for,
        );

        let fixings = 4u32;
        let pairs = 1_000u32;

        // TARF: Monte-Carlo. SDK price == celnet-exotics MC at the SAME (seed,
        // pairs) and the same fixing count ⇒ bit-reproducible ⇒ exact agreement;
        // the SDK surfaces the MC standard error.
        let tarf_seed = 0x7A2F_BEEF_u64;
        let tarf_spec = InstrumentSpec::tarf(
            eurusd(),
            celnet_types::Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Sell,
            TarfTerms::new(
                OptionType::Put,
                1.10,
                0.30,
                2.0,
                TarfRedemption::FullGain,
                fixings,
            )
            .monte_carlo(pairs, tarf_seed),
        );
        let tarf_priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&tarf_spec, market, conventions()),
        )
        .await
        .expect("tarf price returns")
        .expect("tarf price succeeds");
        let tarf_oracle = tarf_price(
            &inputs,
            ExTarf {
                strike: 1.10,
                fixings: fixings as usize,
                target: 0.30,
                leverage: 2.0,
                favourable_side: OptionType::Put,
                notional: 1.0,
                redemption: ExRedemptionStyle::FullGain,
            },
            TarfMcConfig {
                pairs: pairs as usize,
                seed: tarf_seed,
            },
        );
        assert!(
            is_close(tarf_priced.greeks.price, tarf_oracle.price, 1e-12, 1e-12),
            "SDK TARF {} != oracle MC {}",
            tarf_priced.greeks.price,
            tarf_oracle.price
        );
        assert!(
            tarf_priced
                .price_std_error
                .expect("TARF (MC) must surface a std-error through the SDK price")
                > 0.0
        );

        // MC-honesty across the QUOTE path (the GUI live-WS + Excel path): a TARF
        // `Quote` MUST carry `price_std_error`.
        let tarf_quote = tokio::time::timeout(
            STEP_DEADLINE,
            client
                .request_quote(tarf_spec.clone(), conventions())
                .request(),
        )
        .await
        .expect("tarf quote returns")
        .expect("tarf quote succeeds");
        assert!(
            tarf_quote
                .price_std_error
                .expect("TARF QUOTE must surface std-error (WS/SDK quote path)")
                > 0.0
        );

        // Accumulator: Monte-Carlo. SDK price == celnet-exotics MC at the SAME
        // (seed, pairs); surfaces the std-error.
        let acc_seed = 0xACC0_BEEF_u64;
        let acc_spec = InstrumentSpec::accumulator(
            eurusd(),
            celnet_types::Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            AccumulatorTerms::new(1.10, 1.16, 2.0, AccumulatorMonitoring::Discrete, fixings)
                .monte_carlo(pairs, acc_seed),
        );
        let acc_priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&acc_spec, market, conventions()),
        )
        .await
        .expect("accumulator price returns")
        .expect("accumulator price succeeds");
        let acc_oracle = accumulator_price(
            &inputs,
            ExAccumulator {
                pivot: 1.10,
                barrier: 1.16,
                fixings: fixings as usize,
                leverage: 2.0,
                notional: 1.0,
                monitoring: ExMonitoring::Discrete,
            },
            AccumulatorMcConfig {
                pairs: pairs as usize,
                seed: acc_seed,
            },
        );
        assert!(
            is_close(acc_priced.greeks.price, acc_oracle.price, 1e-12, 1e-12),
            "SDK accumulator {} != oracle MC {}",
            acc_priced.greeks.price,
            acc_oracle.price
        );
        assert!(
            acc_priced
                .price_std_error
                .expect("accumulator (MC) must surface a std-error")
                > 0.0
        );

        // Continuous floating lookback: exact closed form, NO std-error, dominates
        // the equivalent vanilla.
        let lb_cont_spec = InstrumentSpec::lookback(
            eurusd(),
            celnet_types::Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            LookbackTerms::continuous(LookbackStyle::Floating, OptionType::Call, 0.0),
        );
        let lb_cont = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&lb_cont_spec, market, conventions()),
        )
        .await
        .expect("continuous lookback price returns")
        .expect("continuous lookback price succeeds");
        let lb_cont_oracle = floating_lookback_price(&inputs, OptionType::Call);
        assert!(
            is_close(lb_cont.greeks.price, lb_cont_oracle, 1e-9, 1e-12),
            "SDK continuous lookback {} != closed form {}",
            lb_cont.greeks.price,
            lb_cont_oracle
        );
        assert!(
            lb_cont.price_std_error.is_none(),
            "a continuous lookback is closed-form and must NOT carry a std-error"
        );
        // ... and its quote must NOT carry a std-error either.
        let lb_cont_quote = tokio::time::timeout(
            STEP_DEADLINE,
            client
                .request_quote(lb_cont_spec.clone(), conventions())
                .request(),
        )
        .await
        .expect("continuous lookback quote returns")
        .expect("continuous lookback quote succeeds");
        assert!(
            lb_cont_quote.price_std_error.is_none(),
            "a closed-form (continuous lookback) quote must NOT carry a std-error"
        );

        // Discrete fixed lookback: Monte-Carlo. SDK price == celnet-exotics MC at
        // the SAME (seed, observations, pairs); surfaces the std-error on both the
        // price and the quote.
        let observations = 16u32;
        let lb_seed = 0x100C_BAC4_u64;
        let lb_disc_spec = InstrumentSpec::lookback(
            eurusd(),
            celnet_types::Tenor::Years(1),
            1.0,
            Quantity::base(1_000_000.0),
            Side::Buy,
            LookbackTerms::discrete(LookbackStyle::Fixed, OptionType::Call, 1.05, observations)
                .monte_carlo(pairs, lb_seed),
        );
        let lb_disc = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&lb_disc_spec, market, conventions()),
        )
        .await
        .expect("discrete lookback price returns")
        .expect("discrete lookback price succeeds");
        let fixed_inputs = VanillaInputs::new(
            market.spot,
            1.05,
            market.vol,
            1.0,
            market.r_dom,
            market.r_for,
        );
        let lb_disc_oracle = lookback_mc(
            &fixed_inputs,
            ExLookback {
                style: ExLookbackStyle::FixedStrike,
                option: OptionType::Call,
            },
            LookbackMcConfig {
                pairs: pairs as usize,
                steps: observations as usize,
                seed: lb_seed,
            },
        );
        assert!(
            is_close(lb_disc.greeks.price, lb_disc_oracle.price, 1e-12, 1e-12),
            "SDK discrete lookback {} != oracle MC {}",
            lb_disc.greeks.price,
            lb_disc_oracle.price
        );
        assert!(
            lb_disc
                .price_std_error
                .expect("a discrete lookback is MC and must surface a std-error")
                > 0.0
        );
        let lb_disc_quote = tokio::time::timeout(
            STEP_DEADLINE,
            client
                .request_quote(lb_disc_spec.clone(), conventions())
                .request(),
        )
        .await
        .expect("discrete lookback quote returns")
        .expect("discrete lookback quote succeeds");
        assert!(
            lb_disc_quote
                .price_std_error
                .expect("discrete-lookback QUOTE must surface std-error")
                > 0.0
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
