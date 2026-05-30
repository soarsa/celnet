//! The `convention` subcommand: resolve and print the FX convention record for a
//! `(pair, tenor)`.
//!
//! Delegates to `celnet_conventions::resolve`, which returns the resolved
//! convention record (delta / ATM / premium styles, cut, day-counts, settlement)
//! together with whether it came from a curated pair profile or a region default.

use celnet_conventions::ResolvedConvention;
use celnet_types::{CcyPair, Tenor};

/// Resolve the convention record for a pair and tenor.
///
/// Total: every pair resolves, falling back to a region default when no curated
/// pair profile exists (the [`ResolvedConvention::source`] records which).
#[must_use]
pub(crate) fn run(pair: CcyPair, tenor: Tenor) -> ResolvedConvention {
    celnet_conventions::resolve(pair, tenor)
}

/// Render a [`ResolvedConvention`] as an aligned key/value report.
#[must_use]
pub(crate) fn format_report(pair: CcyPair, tenor: Tenor, resolved: &ResolvedConvention) -> String {
    let r = &resolved.record;
    let mut out = String::new();
    out.push_str(&format!(
        "convention {pair} {}\n",
        crate::tenor::format_tenor(tenor)
    ));
    out.push_str(&format!("  source              {:?}\n", resolved.source));
    out.push_str(&format!("  delta               {:?}\n", r.delta));
    out.push_str(&format!("  atm                 {:?}\n", r.atm));
    out.push_str(&format!("  premium_style       {:?}\n", r.premium_style));
    out.push_str(&format!("  cut                 {:?}\n", r.cut));
    out.push_str(&format!("  day_count_vol       {:?}\n", r.day_count_vol));
    out.push_str(&format!(
        "  day_count_accr_for  {:?}\n",
        r.day_count_accrual_for
    ));
    out.push_str(&format!(
        "  day_count_accr_dom  {:?}\n",
        r.day_count_accrual_dom
    ));
    out.push_str(&format!("  settlement          {:?}\n", r.settlement));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_same_record_as_underlying() {
        let pair = CcyPair::parse("EURUSD").unwrap();
        let tenor = Tenor::Years(1);
        let r = run(pair, tenor);
        let direct = celnet_conventions::resolve(pair, tenor);
        assert_eq!(r.record, direct.record);
        assert_eq!(r.source, direct.source);
    }

    #[test]
    fn report_mentions_pair_and_tenor() {
        let pair = CcyPair::parse("EURUSD").unwrap();
        let tenor = Tenor::Months(3);
        let report = format_report(pair, tenor, &run(pair, tenor));
        assert!(report.contains("EURUSD"));
        assert!(report.contains("3M"));
        assert!(report.contains("delta"));
    }
}
