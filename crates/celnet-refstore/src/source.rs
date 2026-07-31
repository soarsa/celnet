//! The ingestion adapter traits and the deterministic OSS govvie source (§7.1–§7.2).
//!
//! Every source is a `fetch + parse → canonical record` adapter behind a common trait; it never
//! touches the store directly (the lifecycle / caller does). Three families exist (§7.1): open pull
//! adapters, ISO 15022/20022 message adapters, and the pluggable customer-licensed vendor-CA adapter.
//! This crate ships the **deterministic OSS govvie source** — the complete free govvie/rates path,
//! which *derives* schedule-driven CA events from the open issuance terms already in `celnet-refdata`
//! (no external feed). The corporate universe enters only through a vendor adapter a customer wires to
//! [`CorpActionSource`]; this crate deliberately ships **no bundled commercial data** (guardrail 7).

use celnet_corpactions::{
    BondSchedule, CaDates, CaEvent, CaStatus, CaTerms, Caev, Camv, CivilDate, ScheduleFlow,
};
use celnet_refdata::{CivilYmd, GovBondSpec, government_universe};

use crate::master::{CouponType, ExternalIds, InstrumentMaster, InstrumentTerms, Provenance};
use crate::store::StoredCorpAction;

/// Failure modes of an ingestion source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// A reference record carried terms the schedule primitives rejected.
    InvalidTerms(String),
    /// A reference record carried a date that is not a real calendar date.
    InvalidDate(String),
}

impl core::fmt::Display for SourceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidTerms(id) => write!(f, "invalid instrument terms for {id}"),
            Self::InvalidDate(id) => write!(f, "invalid date on instrument {id}"),
        }
    }
}

impl core::error::Error for SourceError {}

/// A reference-data source: yields mastered instrument records, timestamped `recorded_at`.
pub trait RefDataSource {
    /// A short source label (drives survivorship priority + lineage).
    fn source_id(&self) -> &str;

    /// Fetch + normalize this source's instrument universe as of `recorded_at`.
    ///
    /// # Errors
    /// [`SourceError`] if a record cannot be normalized.
    fn masters(&self, recorded_at: CivilDate) -> Result<Vec<InstrumentMaster>, SourceError>;
}

/// A corporate-action source: yields normalized CA events, timestamped `recorded_at`.
///
/// The govvie source *derives* the deterministic schedule-driven set (coupon INTR + maturity REDM);
/// a customer's vendor adapter implements this to feed the comprehensive corporate set (TEND/EXOF/…)
/// from its licensed MT 564 / seev.031 feed.
pub trait CorpActionSource {
    /// A short source label.
    fn source_id(&self) -> &str;

    /// Fetch + normalize the corporate actions **upcoming after** `recorded_at` (already-settled
    /// events carry no future lifecycle to drive).
    ///
    /// # Errors
    /// [`SourceError`] if a record cannot be normalized.
    fn corp_actions(&self, recorded_at: CivilDate) -> Result<Vec<StoredCorpAction>, SourceError>;
}

/// The source priority the govvie source stamps — above a raw vendor feed, below an official
/// issuer/DMO/Treasury override (§4.3 survivorship default ordering).
const GOVVIE_SOURCE_PRIORITY: u8 = 200;
/// Quality score for a fully-specified, check-valid govvie record.
const GOVVIE_QUALITY: u8 = 100;

/// The deterministic OSS government-bond source: derives mastered records and schedule-driven CA
/// events from the curated open govvie universe (`celnet-refdata`). No IO, no external feed.
#[derive(Debug, Clone, Default)]
pub struct GovvieSource {
    specs: Vec<GovBondSpec>,
}

impl GovvieSource {
    /// The source keyed to the full curated government universe (US/UK/EUR).
    #[must_use]
    pub fn curated() -> Self {
        Self {
            specs: government_universe(),
        }
    }

    /// A source over an explicit spec set (test injection of a targeted universe).
    #[must_use]
    pub fn from_specs(specs: Vec<GovBondSpec>) -> Self {
        Self { specs }
    }

    /// Map a `celnet-refdata` civil date onto the corporate-actions civil date, validating it.
    fn civil(ymd: CivilYmd, ctx: &str) -> Result<CivilDate, SourceError> {
        let month =
            u8::try_from(ymd.month).map_err(|_| SourceError::InvalidDate(ctx.to_string()))?;
        let day = u8::try_from(ymd.day).map_err(|_| SourceError::InvalidDate(ctx.to_string()))?;
        let date = CivilDate::new(ymd.year, month, day);
        if date.is_valid() {
            Ok(date)
        } else {
            Err(SourceError::InvalidDate(ctx.to_string()))
        }
    }

    /// Coupons per year from a `GovBondSpec` frequency label (`0` for a zero / discount security).
    fn coupons_per_year(label: &str) -> u32 {
        match label {
            "annual" => 1,
            "semi_annual" => 2,
            "quarterly" => 4,
            _ => 0,
        }
    }

    /// Derive the full cashflow schedule for a spec. A fixed-coupon bond rolls its coupon dates back
    /// from maturity (from `dated_date`, or — when the curated spec omits it — a conservative
    /// first-accrual floor 40y before maturity so every real coupon is captured; pricing filters the
    /// pre-valuation flows). A zero carries a single redemption flow at maturity.
    fn schedule_for(spec: &GovBondSpec) -> Result<BondSchedule, SourceError> {
        let maturity = Self::civil(spec.maturity_date, &spec.instrument_id)?;
        let freq = Self::coupons_per_year(spec.coupon_frequency);
        if spec.coupon_type == "zero" || freq == 0 {
            let flow = ScheduleFlow {
                date: maturity,
                coupon: 0.0,
                principal: spec.redemption,
            };
            return BondSchedule::new(vec![flow])
                .map_err(|_| SourceError::InvalidTerms(spec.instrument_id.clone()));
        }
        let first_accrual = match spec.dated_date {
            Some(d) => Self::civil(d, &spec.instrument_id)?,
            None => CivilDate::new(maturity.year - 40, 1, 1),
        };
        BondSchedule::fixed_coupon(
            first_accrual,
            maturity,
            spec.coupon_rate,
            freq,
            spec.redemption,
        )
        .map_err(|_| SourceError::InvalidTerms(spec.instrument_id.clone()))
    }

    /// Normalize one spec into a mastered instrument record effective from `recorded_at`.
    fn master_for(
        spec: &GovBondSpec,
        recorded_at: CivilDate,
    ) -> Result<InstrumentMaster, SourceError> {
        let schedule = Self::schedule_for(spec)?;
        let maturity = Self::civil(spec.maturity_date, &spec.instrument_id)?;
        let dated = match spec.dated_date {
            Some(d) => Some(Self::civil(d, &spec.instrument_id)?),
            None => None,
        };
        let coupon_type = if spec.coupon_type == "zero" {
            CouponType::Zero
        } else {
            CouponType::Fixed
        };
        Ok(InstrumentMaster {
            instrument_id: spec.instrument_id.clone(),
            external_ids: ExternalIds {
                figi: None,
                isin: Some(spec.isin.clone()),
                lei_issuer: None,
                cusip: spec.cusip.clone(),
            },
            terms: InstrumentTerms {
                name: spec.name.clone(),
                issuer: spec.issuer.clone(),
                currency: spec.currency.to_string(),
                coupon_type,
                coupon_rate: spec.coupon_rate,
                coupons_per_year: Self::coupons_per_year(spec.coupon_frequency),
                day_count: spec.day_count.to_string(),
                dated_date: dated,
                maturity_date: maturity,
                redemption: spec.redemption,
                calendars: spec.calendars.iter().map(|c| (*c).to_string()).collect(),
            },
            schedule,
            provenance: Provenance {
                source_priority: GOVVIE_SOURCE_PRIORITY,
                source: "treasury_fiscaldata/dmo/ecb (derived)".into(),
                valid_from: recorded_at,
                recorded_at,
                quality: GOVVIE_QUALITY,
            },
        })
    }

    /// Derive the schedule-driven CA events (upcoming after `recorded_at`) for one spec: an INTR for
    /// each future coupon flow and a REDM at maturity — the deterministic MAND set (§3).
    fn corp_actions_for(
        spec: &GovBondSpec,
        recorded_at: CivilDate,
    ) -> Result<Vec<StoredCorpAction>, SourceError> {
        let schedule = Self::schedule_for(spec)?;
        let prov = Provenance {
            source_priority: GOVVIE_SOURCE_PRIORITY,
            source: "derived".into(),
            valid_from: recorded_at,
            recorded_at,
            quality: GOVVIE_QUALITY,
        };
        let mut out = Vec::new();
        for flow in schedule.flows() {
            if flow.date <= recorded_at {
                continue; // an already-settled flow has no upcoming lifecycle
            }
            let is_maturity = flow.principal > 0.0;
            let caev = if is_maturity { Caev::Redm } else { Caev::Intr };
            let terms = if is_maturity {
                CaTerms::full_at_par()
            } else {
                CaTerms::coupon(flow.coupon)
            };
            let dates = CaDates {
                announcement: recorded_at,
                record: flow.date,
                ex: flow.date,
                response_deadline: None,
                payment: flow.date,
            };
            let ca_id = format!(
                "{}:{}:{:04}{:02}{:02}",
                spec.instrument_id,
                caev.code(),
                flow.date.year,
                flow.date.month,
                flow.date.day
            );
            out.push(StoredCorpAction {
                ca_id,
                event: CaEvent {
                    isin: spec.isin.clone(),
                    caev,
                    camv: Camv::Mand,
                    dates,
                    terms,
                    status: CaStatus::Announced,
                    source_ref: format!("derived:{}", spec.instrument_id),
                },
                provenance: prov.clone(),
            });
        }
        Ok(out)
    }
}

impl RefDataSource for GovvieSource {
    fn source_id(&self) -> &str {
        "govvie_deterministic"
    }

    fn masters(&self, recorded_at: CivilDate) -> Result<Vec<InstrumentMaster>, SourceError> {
        self.specs
            .iter()
            .map(|s| Self::master_for(s, recorded_at))
            .collect()
    }
}

impl CorpActionSource for GovvieSource {
    fn source_id(&self) -> &str {
        "govvie_deterministic"
    }

    fn corp_actions(&self, recorded_at: CivilDate) -> Result<Vec<StoredCorpAction>, SourceError> {
        let mut out = Vec::new();
        for spec in &self.specs {
            out.extend(Self::corp_actions_for(spec, recorded_at)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_masters_for_the_whole_curated_universe() {
        let src = GovvieSource::curated();
        let masters = src.masters(CivilDate::new(2026, 4, 16)).expect("masters");
        assert!(
            masters.len() >= 230,
            "universe too small: {}",
            masters.len()
        );
        // Every mastered govvie carries its ISIN and a non-empty schedule.
        for m in &masters {
            assert!(m.isin().is_some(), "missing ISIN for {}", m.instrument_id);
            assert!(
                !m.schedule.is_empty(),
                "empty schedule for {}",
                m.instrument_id
            );
        }
    }

    #[test]
    fn derives_future_coupon_and_redemption_events() {
        let src = GovvieSource::curated();
        let recorded = CivilDate::new(2026, 4, 16);
        let cas = src.corp_actions(recorded).expect("cas");
        // The deterministic set is only INTR + REDM, all mandatory, all in the future.
        assert!(cas.iter().any(|c| c.event.caev == Caev::Intr));
        assert!(cas.iter().any(|c| c.event.caev == Caev::Redm));
        for c in &cas {
            assert!(matches!(c.event.caev, Caev::Intr | Caev::Redm));
            assert_eq!(c.event.camv, Camv::Mand);
            assert!(c.event.effective_date() > recorded);
        }
    }
}
