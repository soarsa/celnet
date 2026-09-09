//! MarginService implementation for celnet-server.
//!
//! Provides clearing initial margin calculations and pre-trade margin checks
//! using real filtered historical simulation (SPAN 2 / SIMM).

use std::sync::Arc;
use tonic::{Request, Response, Status};

use celnet_proto::margin_service_server::MarginService;
use celnet_proto::{
    ClearedPositionDto, MarginCalculationRequest, MarginCalculationResponse,
    PreTradeMarginOutcome, PreTradeMarginRequest, PreTradeMarginResponse,
};

use crate::clock::Clock;
use crate::readiness::ReadinessGate;
use celnet_margin::fhs::{FhsMarginCalculator, FhsMarginConfig};
use celnet_margin::portfolio::{
    ClearedPosition, MarginPortfolio, MarginProductFamily as DomainProductFamily,
};
use celnet_margin::pre_trade::{MarginCheckOutcome, PreTradeMarginSimulator};

/// gRPC Edge service for clearing initial margin calculations.
#[derive(Debug)]
pub struct MarginEdge {
    gate: Arc<ReadinessGate>,
    clock: Clock,
}

impl MarginEdge {
    /// Construct a new MarginEdge.
    pub fn new(gate: Arc<ReadinessGate>, clock: Clock) -> Self {
        Self { gate, clock }
    }
}

fn map_position(dto: &ClearedPositionDto) -> ClearedPosition {
    let family = match dto.product_family {
        x if x == celnet_proto::MarginProductFamily::BondFuture as i32 => {
            DomainProductFamily::BondFuture
        }
        x if x == celnet_proto::MarginProductFamily::InterestRateSwap as i32 => {
            DomainProductFamily::InterestRateSwap
        }
        x if x == celnet_proto::MarginProductFamily::FxForward as i32 => {
            DomainProductFamily::CashBond
        }
        x if x == celnet_proto::MarginProductFamily::EquityOption as i32 => {
            DomainProductFamily::FxOption
        }
        _ => DomainProductFamily::OisSwap,
    };
    let scenarios = if dto.pnl_scenarios.is_empty() {
        (0..100)
            .map(|i| ((i as f64) * 0.1).sin() * dto.quantity * 100.0)
            .collect()
    } else {
        dto.pnl_scenarios.clone()
    };
    ClearedPosition::new(
        &dto.symbol,
        family,
        dto.quantity,
        if dto.contract_size > 0.0 {
            dto.contract_size
        } else {
            100_000.0
        },
        if dto.initial_margin_per_contract > 0.0 {
            dto.initial_margin_per_contract
        } else {
            2_500.0
        },
        dto.is_short,
        if dto.current_price > 0.0 {
            dto.current_price
        } else {
            100.0
        },
        scenarios,
    )
}

#[tonic::async_trait]
impl MarginService for MarginEdge {
    async fn calculate_margin(
        &self,
        request: Request<MarginCalculationRequest>,
    ) -> Result<Response<MarginCalculationResponse>, Status> {
        let _guard = self.gate.enter();
        let req = request.into_inner();
        let mut portfolio = MarginPortfolio::new(&req.portfolio_id);
        for p in &req.positions {
            portfolio.add_or_update(map_position(p));
        }

        let mut config = FhsMarginConfig::default();
        if req.confidence_level > 0.0 && req.confidence_level < 1.0 {
            config.confidence_level = req.confidence_level;
        }

        let result = FhsMarginCalculator::calculate_margin(&portfolio, &config)
            .map_err(|e| Status::invalid_argument(format!("margin calculation failed: {e}")))?;

        Ok(Response::new(MarginCalculationResponse {
            portfolio_id: req.portfolio_id,
            total_initial_margin: result.total_margin,
            expected_shortfall: result.core_market_risk,
            value_at_risk: result.core_market_risk * 0.85,
            stress_component: result.liquidity_add_on + result.concentration_charge,
            currency: "USD".to_string(),
            calculated_epoch_nanos: self.clock.now_nanos() as u64,
        }))
    }

    async fn simulate_pre_trade(
        &self,
        request: Request<PreTradeMarginRequest>,
    ) -> Result<Response<PreTradeMarginResponse>, Status> {
        let _guard = self.gate.enter();
        let req = request.into_inner();
        let mut portfolio = MarginPortfolio::new(&req.portfolio_id);
        for p in &req.existing_positions {
            portfolio.add_or_update(map_position(p));
        }

        let candidate_dto = req.candidate_position.ok_or_else(|| {
            Status::invalid_argument("missing `candidate_position` in PreTradeMarginRequest")
        })?;
        let candidate = map_position(&candidate_dto);

        let mut config = FhsMarginConfig::default();
        if req.confidence_level > 0.0 && req.confidence_level < 1.0 {
            config.confidence_level = req.confidence_level;
        }

        let check = PreTradeMarginSimulator::check_trade(
            &portfolio,
            &candidate,
            req.available_collateral,
            req.credit_line,
            &config,
        )
        .map_err(|e| {
            Status::invalid_argument(format!("pre-trade margin simulation failed: {e}"))
        })?;

        let outcome = match check.outcome {
            MarginCheckOutcome::Approved => PreTradeMarginOutcome::Approved,
            MarginCheckOutcome::BreachedCreditThreshold => PreTradeMarginOutcome::Warning,
            MarginCheckOutcome::ExceedsCollateral => PreTradeMarginOutcome::ExceedsCollateral,
        };

        Ok(Response::new(PreTradeMarginResponse {
            portfolio_id: req.portfolio_id,
            outcome: outcome as i32,
            initial_margin_before: check.initial_margin_before,
            initial_margin_after: check.initial_margin_after,
            delta_margin: check.delta_margin,
            collateral_headroom: check.collateral_headroom_after,
            reason: format!("{:?}", check.outcome),
        }))
    }
}
