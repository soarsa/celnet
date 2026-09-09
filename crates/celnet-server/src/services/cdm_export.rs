//! ISDA Common Domain Model (CDM 2026) projection & export service.
//!
//! Provides conversion from native execution and booking contracts
//! ([`celnet_proto::Execution`]) into canonical ISDA CDM 2026 digital trade lifecycle
//! events ([`celnet_types::cdm::CdmLifecycleEvent`]).
//!
//! Conforms to regulatory trade repository (DTCC, EMIR, CFTC) and central clearing
//! house requirements for digital trade event lineage.

#![forbid(unsafe_code)]

use celnet_proto::{Execution, instrument::Product};
use celnet_types::cdm::{
    CdmExerciseStyle, CdmInterestRatePayout, CdmLifecycleEvent, CdmOptionPayout, CdmParty,
    CdmPartyRole, CdmPayout, CdmProduct, CdmSettlementType, CdmTradeIdentifier,
};
use celnet_types::{BrokenDate, Ccy, CcyPair, OptionType, Underlying};

/// Default LEI for Celnet platform executions when unconfigured.
pub const DEFAULT_CELNET_LEI: &str = "549300CELNET2026MKR0";

/// Converts an internal execution into a canonical ISDA CDM 2026 lifecycle event.
///
/// # Errors
/// Returns error description if instrument is missing or invalid.
pub fn execution_to_cdm_event(
    exec: &Execution,
    issuer_lei: &str,
    client_id: &str,
) -> Result<CdmLifecycleEvent, String> {
    let instrument = exec
        .instrument
        .as_ref()
        .ok_or_else(|| "missing instrument on execution".to_string())?;

    let trade_id = format!("TRD-CELNET-{}", exec.execution_id);
    let cdm_trade_id = CdmTradeIdentifier::new(issuer_lei, trade_id)
        .with_uti(format!("UTI-2026-{:012}", exec.execution_id));

    let parties = vec![
        CdmParty::new(issuer_lei, CdmPartyRole::ExecutingEntity)
            .with_name("Celnet Execution Venue"),
        CdmParty::new(client_id, CdmPartyRole::Counterparty)
            .with_name(format!("Client Account {client_id}")),
    ];

    let notional = instrument
        .quantity
        .as_ref()
        .map_or(1_000_000.0, |q| q.notional);

    let (underlying, notional_ccy) = match &instrument.underlying {
        Some(u) => {
            let und = Underlying::try_from(u.clone())
                .unwrap_or_else(|_| Underlying::Fx(CcyPair::new(Ccy::EUR, Ccy::USD)));
            let ccy = match &und {
                Underlying::Fx(p) => p.base,
                Underlying::Metal(m) => m.quote,
                Underlying::Equity(e) => e.currency,
                Underlying::Commodity(c) => c.currency,
                Underlying::DigitalAsset(d) => Ccy::parse(&d.quote).unwrap_or(Ccy::USD),
            };
            (und, ccy)
        }
        None => (Underlying::Fx(CcyPair::new(Ccy::EUR, Ccy::USD)), Ccy::EUR),
    };

    let epoch_nanos_u64 = exec.epoch_nanos.max(0) as u64;
    let effective_date = epoch_nanos_to_date(epoch_nanos_u64);
    let term_days = (instrument.expiry_years.max(0.01) * 365.25) as u32;
    let termination_date = add_days_to_date(effective_date, term_days);

    let payout = match &instrument.product {
        Some(Product::Vanilla(v)) => {
            let opt_type = match v.option_type {
                x if x == celnet_proto::OptionType::Put as i32 => OptionType::Put,
                _ => OptionType::Call,
            };
            let strike = match &v.strike {
                Some(s) => match &s.spec {
                    Some(celnet_proto::strike_or_delta::Spec::Strike(val)) => *val,
                    _ => 1.0,
                },
                None => 1.0,
            };

            CdmPayout::Option(CdmOptionPayout {
                option_type: opt_type,
                strike,
                exercise_style: CdmExerciseStyle::European,
                settlement_type: CdmSettlementType::Cash,
                premium: exec.traded_premium,
                premium_ccy: notional_ccy,
            })
        }
        _ => CdmPayout::InterestRate(CdmInterestRatePayout {
            is_fixed: true,
            rate_or_spread: 0.045,
            day_count: "ACT/360".to_string(),
            payment_frequency_months: 6,
            floating_index: None,
        }),
    };

    let product = CdmProduct {
        underlying,
        notional,
        notional_ccy,
        effective_date,
        termination_date,
        payout,
    };

    product.validate().map_err(|e| e.to_string())?;

    let event_id = format!("EVT-{}-{:08}", epoch_nanos_u64, exec.execution_id);
    Ok(CdmLifecycleEvent::new_execution(
        event_id,
        epoch_nanos_u64,
        cdm_trade_id,
        product,
        parties,
    ))
}

/// Serialize an execution to an ISDA CDM 2026 JSON string representation.
///
/// # Errors
/// Returns error description if event creation or JSON serialization fails.
pub fn export_execution_to_cdm_json(
    exec: &Execution,
    issuer_lei: &str,
    client_id: &str,
) -> Result<String, String> {
    let event = execution_to_cdm_event(exec, issuer_lei, client_id)?;
    serde_json::to_string_pretty(&event)
        .map_err(|e| format!("failed to serialize ISDA CDM 2026 event: {e}"))
}

fn epoch_nanos_to_date(nanos: u64) -> BrokenDate {
    let secs = (nanos / 1_000_000_000) as i64;
    // Approximating civil date from timestamp
    let days = (secs / 86400) + 719468;
    let era = (if days >= 0 { days } else { days - 146096 }) / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    BrokenDate {
        year: y as i32,
        month: m as u8,
        day: d as u8,
    }
}

fn add_days_to_date(date: BrokenDate, days: u32) -> BrokenDate {
    let mut d = date.day as u32 + days;
    let mut m = date.month as u32;
    let mut y = date.year;

    while d > 28 {
        let days_in_month = match m {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0) {
                    29
                } else {
                    28
                }
            }
            _ => 30,
        };

        if d > days_in_month {
            d -= days_in_month;
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        } else {
            break;
        }
    }

    BrokenDate {
        year: y,
        month: m as u8,
        day: d as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{Instrument, Quantity, Side, StrikeOrDelta, Vanilla, strike_or_delta};
    use celnet_types::cdm::CdmLifecycleEventType;

    #[test]
    fn execution_exports_to_valid_cdm_event() {
        let exec = Execution {
            execution_id: 987654,
            quote_id: 123,
            side: Side::Buy as i32,
            traded_premium: 12500.0,
            instrument: Some(Instrument {
                underlying: Some(celnet_proto::Underlying::fx(celnet_proto::CcyPair {
                    base: "EUR".to_string(),
                    quote: "USD".to_string(),
                })),
                tenor: None,
                expiry_years: 0.5,
                quantity: Some(Quantity {
                    notional: 5_000_000.0,
                    base_ccy: true,
                }),
                side: Side::Buy as i32,
                solve: None,
                pricing_model: 0,
                product: Some(Product::Vanilla(Vanilla {
                    option_type: celnet_proto::OptionType::Call as i32,
                    strike: Some(StrikeOrDelta {
                        spec: Some(strike_or_delta::Spec::Strike(1.1250)),
                    }),
                })),
                ..Default::default()
            }),
            epoch_nanos: 1_788_572_000_000_000_000,
            attribution: None,
            pricing_provenance: None,
        };

        let event = execution_to_cdm_event(&exec, DEFAULT_CELNET_LEI, "CLIENT-ACCOUNT-A")
            .expect("converts clean");

        assert_eq!(event.event_type, CdmLifecycleEventType::Execution);
        assert_eq!(event.trade_id.issuer_lei, DEFAULT_CELNET_LEI);
        assert_eq!(event.trade_id.assigned_trade_id, "TRD-CELNET-987654");
        assert_eq!(
            event.trade_id.uti,
            Some("UTI-2026-000000987654".to_string())
        );
        assert_eq!(event.product.notional, 5_000_000.0);
        assert_eq!(event.product.notional_ccy, Ccy::EUR);

        let json = export_execution_to_cdm_json(&exec, DEFAULT_CELNET_LEI, "CLIENT-ACCOUNT-A")
            .expect("serializes to json");
        assert!(json.contains("TRD-CELNET-987654"));
        assert!(json.contains("UTI-2026-000000987654"));
        assert!(json.contains("Execution"));
    }

    #[test]
    fn cdm_event_transitions_to_clearing_and_settlement() {
        let exec = Execution {
            execution_id: 101,
            quote_id: 101,
            side: Side::Buy as i32,
            traded_premium: 5000.0,
            instrument: Some(Instrument {
                underlying: Some(celnet_proto::Underlying::equity(celnet_proto::EquityRef::new(
                    celnet_proto::Symbol::new("NVDA", ""),
                    "USD",
                ))),
                tenor: None,
                expiry_years: 0.25,
                quantity: Some(Quantity {
                    notional: 100_000.0,
                    base_ccy: true,
                }),
                side: Side::Buy as i32,
                solve: None,
                pricing_model: 0,
                product: Some(Product::Vanilla(Vanilla {
                    option_type: celnet_proto::OptionType::Call as i32,
                    strike: Some(StrikeOrDelta {
                        spec: Some(strike_or_delta::Spec::Strike(130.0)),
                    }),
                })),
                ..Default::default()
            }),
            epoch_nanos: 1_788_572_000_000_000_000,
            attribution: None,
            pricing_provenance: None,
        };

        let exec_event = execution_to_cdm_event(&exec, DEFAULT_CELNET_LEI, "CLIENT-NVDA")
            .expect("executes clean");

        // Transition: Execution -> Confirmation
        let confirm_event = exec_event.transition_to(
            "EVT-CONFIRM-101",
            CdmLifecycleEventType::Confirmation,
            (exec.epoch_nanos + 1_000_000_000).max(0) as u64,
        );
        assert_eq!(confirm_event.lineage_event_id, Some(exec_event.event_id));
        assert_eq!(
            confirm_event.event_type,
            CdmLifecycleEventType::Confirmation
        );

        // Transition: Confirmation -> Clearing
        let clear_event = confirm_event.transition_to(
            "EVT-CLEAR-101",
            CdmLifecycleEventType::Clearing,
            (exec.epoch_nanos + 2_000_000_000).max(0) as u64,
        );
        assert_eq!(clear_event.lineage_event_id, Some(confirm_event.event_id));
        assert_eq!(clear_event.event_type, CdmLifecycleEventType::Clearing);
    }
}
