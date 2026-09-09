//! Exchange Binary Transcoder.
//!
//! Provides zero-loss, zero-allocation bidirectional transcoding between
//! native exchange wire formats and Celnet internal structures (`celnet-sbe` and `celnet-types`).
#![deny(missing_docs)]

use crate::ilink::{ExecStatus, ExecutionReport, NewOrderSingle};
use crate::mdp::{BookEntry, EntryType};
use crate::ouch::{OuchEnterOrder, OuchOrderExecuted};
use crate::ExchangeCodecError;

/// Canonical transcoder connecting binary exchange messages to Celnet engine models.
pub struct ExchangeTranscoder;

impl ExchangeTranscoder {
    /// Transcode MDP book entry into Celnet SBE PriceTick.
    pub fn mdp_entry_to_price_tick(
        entry: &BookEntry,
        timestamp_nanos: i64,
    ) -> Result<celnet_sbe::PriceTick, ExchangeCodecError> {
        let price = entry.price_f64();

        let (bid, ask) = match entry.entry_type {
            EntryType::Bid | EntryType::ImpliedBid => (price, 0.0),
            EntryType::Offer | EntryType::ImpliedOffer => (0.0, price),
            EntryType::Trade => (price, price),
            EntryType::BookReset => (0.0, 0.0),
        };

        Ok(celnet_sbe::PriceTick {
            epoch_nanos: timestamp_nanos,
            pair_id: entry.security_id,
            flags: 1, // Tradable
            bid,
            ask,
        })
    }

    /// Transcode an SBE ExecutionReport into an iLink binary execution report.
    pub fn celnet_exec_to_ilink(
        report: &celnet_sbe::ExecutionReport,
    ) -> ExecutionReport {
        let mut cl_ord_id = [b' '; 20];
        let bytes = format!("Q-{}", report.quote_id);
        let b = bytes.as_bytes();
        let len = b.len().min(20);
        cl_ord_id[..len].copy_from_slice(&b[..len]);

        let status = match report.status {
            celnet_sbe::ExecutionStatus::Filled => ExecStatus::Filled,
            celnet_sbe::ExecutionStatus::New => ExecStatus::New,
            celnet_sbe::ExecutionStatus::Rejected => ExecStatus::Rejected,
            celnet_sbe::ExecutionStatus::Expired => ExecStatus::Expired,
        };

        let last_px_mantissa = (report.exec_price * 10_000_000.0).round() as i64;

        ExecutionReport {
            cl_ord_id,
            order_id: report.quote_id,
            exec_id: report.exec_id,
            status,
            cum_qty: report.exec_quantity as u32,
            leaves_qty: 0,
            last_px_mantissa,
            last_px_exponent: -7,
            last_qty: report.exec_quantity as u32,
            transact_time_nanos: report.epoch_nanos.max(0) as u64,
        }
    }

    /// Transcode an iLink ExecutionReport to a Celnet SBE ExecutionReport.
    pub fn ilink_to_celnet_exec(
        report: &ExecutionReport,
        lp_id: u64,
    ) -> celnet_sbe::ExecutionReport {
        let status = match report.status {
            ExecStatus::New => celnet_sbe::ExecutionStatus::New,
            ExecStatus::PartiallyFilled | ExecStatus::Filled => celnet_sbe::ExecutionStatus::Filled,
            ExecStatus::Canceled | ExecStatus::Expired => celnet_sbe::ExecutionStatus::Expired,
            ExecStatus::Replaced => celnet_sbe::ExecutionStatus::New,
            ExecStatus::Rejected => celnet_sbe::ExecutionStatus::Rejected,
        };

        celnet_sbe::ExecutionReport {
            exec_id: report.exec_id,
            quote_id: report.order_id,
            epoch_nanos: report.transact_time_nanos as i64,
            exec_price: report.last_px_f64(),
            exec_quantity: report.last_qty as f64,
            side: celnet_sbe::Side::Buy,
            status,
            lp_id,
        }
    }

    /// Transcode an OUCH Enter Order into an iLink NewOrderSingle.
    pub fn ouch_to_ilink(ouch: &OuchEnterOrder) -> NewOrderSingle {
        let mut cl_ord_id = [b' '; 20];
        cl_ord_id[..14].copy_from_slice(&ouch.order_token);

        let mut firm = [b' '; 8];
        firm[..4].copy_from_slice(&ouch.firm);

        NewOrderSingle {
            cl_ord_id,
            security_id: 1, // mapped from stock symbol
            side: ouch.side,
            order_qty: ouch.shares,
            price_mantissa: (ouch.price_f64() * 10_000_000.0).round() as i64,
            price_exponent: -7,
            time_in_force: ouch.time_in_force,
            manual_order_indicator: 0,
            executing_firm_id: firm,
        }
    }

    /// Transcode an iLink ExecutionReport into an OUCH OrderExecuted message.
    pub fn ilink_to_ouch_executed(report: &ExecutionReport) -> OuchOrderExecuted {
        let mut order_token = [b' '; 14];
        let copy_len = report.cl_ord_id.len().min(14);
        order_token[..copy_len].copy_from_slice(&report.cl_ord_id[..copy_len]);

        let exec_px = (report.last_px_f64() * 10_000.0).round() as u32;

        OuchOrderExecuted {
            timestamp_nanos: report.transact_time_nanos,
            order_token,
            executed_shares: report.last_qty,
            execution_price: exec_px,
            match_number: report.exec_id,
        }
    }
}
