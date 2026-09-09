//! AlgoExecutionService implementation for celnet-server.
//!
//! Provides algorithmic order slicing (TWAP, Almgren-Chriss Optimal Liquidation)
//! and real-time execution tracking.

use std::sync::{Arc, Mutex};
use tonic::{Request, Response, Status};

use celnet_proto::algo_execution_service_server::AlgoExecutionService;
use celnet_proto::{
    AlgoOrderResponse, AlgoOrderStatus, AlgoPeggingStyle, AlgoStrategyType, ChildSliceDto,
    ChildSliceStatus, GetAlgoOrderRequest, ListAlgoOrdersRequest, ListAlgoOrdersResponse,
    RecordAlgoFillRequest, SubmitAlgoOrderRequest,
};

use crate::clock::Clock;
use crate::readiness::ReadinessGate;
use celnet_algo::engine::{AlgoParentOrder, ChildSliceStatus as DomainSliceStatus, ParentOrderStatus as DomainOrderStatus};
use celnet_algo::optimal::OptimalExecutionConfig;
use celnet_algo::twap::TwapConfig;
use celnet_algo::PeggingStyle;

/// gRPC Edge service for algorithmic order decomposition and tracking.
#[derive(Debug)]
pub struct AlgoEdge {
    gate: Arc<ReadinessGate>,
    clock: Clock,
    orders: Arc<Mutex<Vec<AlgoParentOrder>>>,
}

impl AlgoEdge {
    /// Construct a new AlgoEdge.
    pub fn new(gate: Arc<ReadinessGate>, clock: Clock) -> Self {
        Self {
            gate,
            clock,
            orders: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

fn map_order_to_response(order: &AlgoParentOrder, client_id: &str, is_buy: bool, created_nanos: u64) -> AlgoOrderResponse {
    let status = match order.status {
        DomainOrderStatus::Active => AlgoOrderStatus::Active,
        DomainOrderStatus::Paused => AlgoOrderStatus::Pending,
        DomainOrderStatus::Completed => AlgoOrderStatus::Completed,
        DomainOrderStatus::Cancelled => AlgoOrderStatus::Cancelled,
    };

    let slices = order
        .slices
        .iter()
        .enumerate()
        .map(|(idx, s)| {
            let slice_status = match s.status {
                DomainSliceStatus::Pending => ChildSliceStatus::Pending,
                DomainSliceStatus::Routed | DomainSliceStatus::PartiallyFilled => ChildSliceStatus::Dispatched,
                DomainSliceStatus::Filled => ChildSliceStatus::Filled,
                DomainSliceStatus::Cancelled => ChildSliceStatus::Cancelled,
            };
            ChildSliceDto {
                slice_index: idx as u32,
                scheduled_offset_seconds: s.scheduled_offset_sec,
                target_quantity: s.target_quantity,
                filled_quantity: s.filled_quantity,
                avg_fill_price: s.avg_fill_price,
                status: slice_status as i32,
            }
        })
        .collect();

    AlgoOrderResponse {
        parent_order_id: order.order_id.clone(),
        client_order_id: client_id.to_string(),
        symbol: order.instrument.clone(),
        total_quantity: order.total_quantity,
        executed_quantity: order.executed_quantity,
        arrival_price: order.arrival_price,
        avg_exec_price: order.avg_exec_price,
        is_buy,
        status: status as i32,
        implementation_shortfall_bps: order.implementation_shortfall_bps(is_buy),
        slices,
        created_epoch_nanos: created_nanos,
    }
}

#[tonic::async_trait]
impl AlgoExecutionService for AlgoEdge {
    async fn submit_algo_order(
        &self,
        request: Request<SubmitAlgoOrderRequest>,
    ) -> Result<Response<AlgoOrderResponse>, Status> {
        let _guard = self.gate.enter();
        let req = request.into_inner();

        let parent_id = format!("ALGO-{}", self.clock.now_nanos());
        let parent_order = match req.strategy_type {
            x if x == AlgoStrategyType::AlgoStrategyOptimalLiquidation as i32 => {
                let opt = req.optimal.unwrap_or(celnet_proto::OptimalLiquidationConfigDto {
                    horizon_seconds: 1800.0,
                    step_count: 5,
                    volatility: 0.20,
                    risk_aversion: 1e-5,
                    temp_impact_eta: 2e-6,
                    perm_impact_gamma: 1e-7,
                });
                let config = OptimalExecutionConfig {
                    horizon_seconds: opt.horizon_seconds.max(10.0),
                    step_count: (opt.step_count as usize).max(1),
                    volatility: opt.volatility.max(0.01),
                    risk_aversion: opt.risk_aversion.max(1e-8),
                    temp_impact_eta: opt.temp_impact_eta.max(1e-8),
                    perm_impact_gamma: opt.perm_impact_gamma.max(1e-8),
                };
                AlgoParentOrder::new_optimal_liquidation(
                    &parent_id,
                    &req.symbol,
                    req.total_quantity,
                    req.arrival_price,
                    &config,
                )
                .map_err(|e| Status::invalid_argument(format!("failed to init optimal liquidation: {e}")))?
            }
            _ => {
                let twap_dto = req.twap.unwrap_or(celnet_proto::TwapConfigDto {
                    duration_seconds: 900.0,
                    slice_count: 5,
                    jitter_factor: 0.05,
                    pegging_style: AlgoPeggingStyle::AlgoPeggingMidpoint as i32,
                });
                let peg = match twap_dto.pegging_style {
                    x if x == AlgoPeggingStyle::AlgoPeggingPrimary as i32 => PeggingStyle::PassiveTouch,
                    x if x == AlgoPeggingStyle::AlgoPeggingMarket as i32 => PeggingStyle::AggressiveSweep,
                    _ => PeggingStyle::Midpoint,
                };
                let config = TwapConfig {
                    duration_seconds: twap_dto.duration_seconds.max(5.0),
                    slice_count: (twap_dto.slice_count as usize).max(1),
                    jitter_factor: twap_dto.jitter_factor.clamp(0.0, 0.5),
                    pegging_style: peg,
                };
                AlgoParentOrder::new_twap(
                    &parent_id,
                    &req.symbol,
                    req.total_quantity,
                    req.arrival_price,
                    &config,
                )
                .map_err(|e| Status::invalid_argument(format!("failed to init twap order: {e}")))?
            }
        };

        let created_nanos = self.clock.now_nanos() as u64;
        let resp = map_order_to_response(&parent_order, &req.client_order_id, req.is_buy, created_nanos);
        {
            let mut guard = self.orders.lock().unwrap();
            guard.push(parent_order);
        }

        Ok(Response::new(resp))
    }

    async fn get_algo_order(
        &self,
        request: Request<GetAlgoOrderRequest>,
    ) -> Result<Response<AlgoOrderResponse>, Status> {
        let _guard = self.gate.enter();
        let req = request.into_inner();
        let guard = self.orders.lock().unwrap();
        let order = guard
            .iter()
            .find(|o| o.order_id == req.parent_order_id)
            .ok_or_else(|| Status::not_found(format!("order {} not found", req.parent_order_id)))?;

        Ok(Response::new(map_order_to_response(
            order,
            &order.order_id,
            true,
            self.clock.now_nanos() as u64,
        )))
    }

    async fn record_algo_fill(
        &self,
        request: Request<RecordAlgoFillRequest>,
    ) -> Result<Response<AlgoOrderResponse>, Status> {
        let _guard = self.gate.enter();
        let req = request.into_inner();
        let mut guard = self.orders.lock().unwrap();
        let order = guard
            .iter_mut()
            .find(|o| o.order_id == req.parent_order_id)
            .ok_or_else(|| Status::not_found(format!("order {} not found", req.parent_order_id)))?;

        order
            .record_fill(req.slice_index as usize, req.fill_quantity, req.fill_price)
            .map_err(|e| Status::invalid_argument(format!("failed to record fill: {e}")))?;

        Ok(Response::new(map_order_to_response(
            order,
            &order.order_id,
            true,
            self.clock.now_nanos() as u64,
        )))
    }

    async fn list_algo_orders(
        &self,
        request: Request<ListAlgoOrdersRequest>,
    ) -> Result<Response<ListAlgoOrdersResponse>, Status> {
        let _guard = self.gate.enter();
        let req = request.into_inner();
        let guard = self.orders.lock().unwrap();
        let orders = guard
            .iter()
            .filter(|o| {
                if req.symbol_filter.is_empty() {
                    true
                } else {
                    o.instrument.contains(&req.symbol_filter)
                }
            })
            .map(|o| map_order_to_response(o, &o.order_id, true, self.clock.now_nanos() as u64))
            .collect();

        Ok(Response::new(ListAlgoOrdersResponse { orders }))
    }
}
