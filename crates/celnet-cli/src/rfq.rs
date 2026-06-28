//! The `rfq` subcommand: the multi-dealer (RFQ-to-many) panel workflow against a
//! running edge, driven through the **same** typed [`celnet_client`] SDK methods
//! the GUI ticket and the SDK examples call (the api-first client-parity rule) —
//! [`Client::request_multi_dealer_quote`](celnet_client::Client::request_multi_dealer_quote)
//! for the ranked panel and
//! [`MultiDealerRfq::accept_dealer`](celnet_client::MultiDealerRfq::accept_dealer)
//! for a pinned-row booking.
//!
//! `rfq` fans one RFQ across the edge's LP panel and prints the ranked dealer
//! ladder — one `dealer` row per responding LP carrying its `lp_id`, its firm
//! bid/offer, the remaining last-look countdown, and the engine-ranked
//! `BEST_BID` / `BEST_OFFER` markers — then, with `--accept <LP_ID>`, books that
//! pinned row (`--side` picks the direction) and prints the execution. Every
//! number is the SERVER's panel surfaced verbatim: the bid/offer/strike/premium
//! columns print with `{}` (Rust's shortest exact round-trip `f64` formatting),
//! so parsing a printed price recovers the SDK's number **bit-for-bit** — the
//! cross-client conformance gate in `tests/conformance.rs` asserts exactly that
//! against an in-process multi-dealer edge.
//!
//! **Honest boundary:** the panel beyond the native maker is the edge's
//! configured deterministic **synthetic** demo/test dealers (labeled
//! `SYNTH-LP-k`) — live LP connectivity/fills are an environment concern, never
//! claimed by the CLI.

use std::fmt::Write as _;

use celnet_client::{
    Conventions, DealerQuote, Execution, InstrumentSpec, Quantity, RankedPanel, Side, StrikeSpec,
};
use celnet_types::{CcyPair, OptionType, Tenor};

use crate::risk::{RiskError, block_on, bounded, connect};

/// Buy or sell on the command line for a panel-row accept: buy lifts the chosen
/// row's offer, sell hits its bid. (A panel *request* is always two-way; only the
/// accept is directional, so `two-way` is not a spelling here.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum CliAcceptSide {
    /// Buy: lift the chosen row's offer.
    Buy,
    /// Sell: hit the chosen row's bid.
    Sell,
}

impl From<CliAcceptSide> for Side {
    fn from(v: CliAcceptSide) -> Self {
        match v {
            CliAcceptSide::Buy => Side::Buy,
            CliAcceptSide::Sell => Side::Sell,
        }
    }
}

/// A fully-parsed `rfq` request: fan one two-way vanilla RFQ across the edge's LP
/// panel, print the ranked ladder, and optionally book one pinned row.
#[derive(Debug, Clone)]
pub(crate) struct RfqReq {
    pub(crate) endpoint: String,
    pub(crate) pair: CcyPair,
    pub(crate) tenor: Tenor,
    pub(crate) expiry_years: f64,
    pub(crate) option: OptionType,
    pub(crate) strike: StrikeSpec,
    pub(crate) notional_base: f64,
    /// Book this `lp_id`'s pinned panel row after the ladder prints; `None`
    /// prints the ranked panel only.
    pub(crate) accept: Option<String>,
    /// The accept direction (ignored without `accept`).
    pub(crate) side: Side,
    /// The `AuthService.Login`-issued session token to authenticate the RFQ as that
    /// user (item B §2); `None` ⇒ the SDK's audited grant-all principal default, the
    /// same posture `stream` uses.
    pub(crate) session_token: Option<String>,
}

/// Run `rfq`: request the ranked multi-dealer panel through the SDK, print the
/// dealer ladder, and — when `--accept <LP_ID>` named a row — book that pinned
/// line and print the execution.
pub(crate) fn run<W: std::io::Write>(req: &RfqReq, out: &mut W) -> Result<(), RiskError> {
    let report = block_on(async {
        let client = connect(&req.endpoint).await?;
        // Authenticate the RFQ as the Login-issued user when a token is supplied;
        // otherwise the SDK asserts the audited grant-all principal the production
        // `Enforce` edge admits (parity with `stream` / the risk commands). The token
        // rides in every QuoteRequest/QuoteAccept body so request + accept agree.
        let client = match &req.session_token {
            Some(token) => client.with_session_token(token.clone()),
            None => client,
        };
        let instrument = InstrumentSpec::vanilla(
            req.pair,
            req.tenor,
            req.expiry_years,
            Quantity::base(req.notional_base),
            Side::TwoWay,
            req.option,
            req.strike,
        );
        let md = client.request_multi_dealer_quote(instrument, Conventions::major_default());
        let panel = bounded("request_multi_dealer_quote", md.request()).await?;
        let mut report = format_panel(&panel, unix_now_nanos());
        if let Some(lp_id) = &req.accept {
            // The server refuses a line that was never on this panel
            // (failed_precondition); guard locally first so the operator's error
            // names the rows the ladder actually showed.
            if panel.dealer(lp_id).is_none() {
                let rows: Vec<&str> = panel.dealers.iter().map(|d| d.lp_id.as_str()).collect();
                return Err(RiskError::Invalid(format!(
                    "--accept {lp_id}: no such dealer row on panel #{} (rows: {})",
                    panel.quote_id,
                    rows.join(", ")
                )));
            }
            let exec = bounded(
                "accept_quote",
                md.accept_dealer(&panel, req.side, lp_id.as_str()),
            )
            .await?;
            report.push_str(&format_execution(&exec, lp_id));
        }
        Ok(report)
    })??;
    out.write_all(report.as_bytes())
        .map_err(|e| RiskError::Invalid(e.to_string()))
}

/// Wall-clock nanoseconds since the Unix epoch. The panel rows' last-look
/// deadlines are epoch-anchored UTC instants, so the printed countdown measures
/// against the same clock the server stamped them with.
fn unix_now_nanos() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
}

/// The remaining last-look window of one row at `now_nanos`, as the ladder prints
/// it: a live countdown in seconds, or `expired` once an accept would be refused.
fn countdown(d: &DealerQuote, now_nanos: i64) -> String {
    match d.last_look_remaining(now_nanos) {
        Some(remaining) => format!("{:.3}s", remaining.as_secs_f64()),
        None => "expired".to_owned(),
    }
}

// ===========================================================================
// formatting — 2-space-indented reports (CLI house style); prices round-trip
// ===========================================================================

/// Format the ranked panel: a header, the resolved line, one `dealer` row per
/// responding LP (lp_id, firm bid/offer, last-look countdown, `native` /
/// `BEST_BID` / `BEST_OFFER` markers), and the ranked touch.
///
/// Bid/offer/strike print via `{}` — the shortest representation that parses
/// back to the identical `f64` bits — so the ladder is both the trader view and
/// the machine-checkable surface of the SDK panel (no precision is shaved off a
/// firm tradable price).
fn format_panel(panel: &RankedPanel, now_nanos: i64) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "rfq panel #{}  dealers={}",
        panel.quote_id,
        panel.dealers.len()
    );
    if let Some(row) = panel.dealers.first() {
        // Every panel row quotes the same resolved line.
        let _ = writeln!(out, "  line strike={}", row.resolved_strike);
    }
    let width = panel
        .dealers
        .iter()
        .map(|d| d.lp_id.len())
        .max()
        .unwrap_or(0);
    for d in &panel.dealers {
        let _ = write!(
            out,
            "  dealer {:<width$}  bid {}  offer {}  last_look {}",
            d.lp_id,
            d.price.bid,
            d.price.offer,
            countdown(d, now_nanos)
        );
        // The native maker's row is the one carrying the edge-priced greeks (an
        // LP discloses a price, not its greeks).
        if d.greeks.is_some() {
            out.push_str("  native");
        }
        if panel.best_bid_lp_id.as_deref() == Some(d.lp_id.as_str()) {
            out.push_str("  BEST_BID");
        }
        if panel.best_offer_lp_id.as_deref() == Some(d.lp_id.as_str()) {
            out.push_str("  BEST_OFFER");
        }
        out.push('\n');
    }
    let bid_touch = panel.best_bid().map_or("none".to_owned(), |b| {
        format!("{} ({})", b.price.bid, b.lp_id)
    });
    let offer_touch = panel.best_offer().map_or("none".to_owned(), |o| {
        format!("{} ({})", o.price.offer, o.lp_id)
    });
    let _ = writeln!(out, "  touch  bid {bid_touch}  offer {offer_touch}");
    out
}

/// Format a pinned-row booking: the execution identity, the booked dealer line,
/// and the traded premium (round-trip precision — the pinned price, never a
/// re-price, so it must equal the printed row to the bit).
fn format_execution(exec: &Execution, lp_id: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "booked execution #{}  quote #{}  side={:?}  vs {}",
        exec.execution_id, exec.quote_id, exec.side, lp_id
    );
    let _ = writeln!(out, "  traded_premium {}", exec.traded_premium);
    if let Some(att) = &exec.attribution {
        let _ = writeln!(out, "  quoted_by {}", att.quoted_by.book);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built two-row panel: a native-marked maker row (greeks would mark
    /// it native on a wire panel; here the marker path is driven by the winner
    /// ids alone) and a synthetic dealer winning the offer.
    fn panel() -> RankedPanel {
        let rows = vec![
            DealerQuote {
                lp_id: "celnet-auto-pricer".to_owned(),
                price: celnet_client::TwoWay {
                    bid: 0.1 + 0.2, // deliberately awkward bits: 0.30000000000000004
                    offer: 0.32,
                },
                greeks: None,
                resolved_strike: 1.12,
                valid_until_nanos: 6_000_000_000,
                attribution: None,
                price_std_error: None,
            },
            DealerQuote {
                lp_id: "SYNTH-LP-2".to_owned(),
                price: celnet_client::TwoWay {
                    bid: 0.29,
                    offer: 0.31,
                },
                greeks: None,
                resolved_strike: 1.12,
                valid_until_nanos: 6_000_000_000,
                attribution: None,
                price_std_error: None,
            },
        ];
        RankedPanel {
            quote_id: 7,
            idempotency_key: "k".to_owned(),
            dealers: rows,
            best_bid_lp_id: Some("celnet-auto-pricer".to_owned()),
            best_offer_lp_id: Some("SYNTH-LP-2".to_owned()),
            conventions: Conventions::major_default(),
            epoch_nanos: 1_000_000_000,
            correlation_id: None,
            surface_version: None,
        }
    }

    /// Parse the first number after `label` on a `line` (test-side mirror of the
    /// conformance harness's column parser).
    fn col(line: &str, label: &str) -> f64 {
        let mut toks = line.split_whitespace();
        toks.by_ref().find(|t| *t == label).expect("label present");
        toks.next().expect("value present").parse().expect("f64")
    }

    #[test]
    fn ladder_prices_round_trip_bit_for_bit() {
        let p = panel();
        let report = format_panel(&p, 1_000_000_000);
        let row = report
            .lines()
            .find(|l| l.trim_start().starts_with("dealer celnet-auto-pricer"))
            .expect("maker row prints");
        // `{}` is shortest-round-trip: parsing the printed bid/offer recovers the
        // panel's exact f64 bits, including the awkward 0.1 + 0.2 sum.
        assert_eq!(col(row, "bid").to_bits(), (0.1_f64 + 0.2).to_bits());
        assert_eq!(col(row, "offer").to_bits(), 0.32_f64.to_bits());
    }

    #[test]
    fn best_markers_sit_on_the_winning_rows_only() {
        let report = format_panel(&panel(), 1_000_000_000);
        let maker = report
            .lines()
            .find(|l| l.contains("celnet-auto-pricer"))
            .unwrap();
        let synth = report.lines().find(|l| l.contains("SYNTH-LP-2")).unwrap();
        assert!(maker.contains("BEST_BID") && !maker.contains("BEST_OFFER"));
        assert!(synth.contains("BEST_OFFER") && !synth.contains("BEST_BID"));
        // The touch line names both winners.
        assert!(report.contains("touch  bid 0.30000000000000004 (celnet-auto-pricer)"));
        assert!(report.contains("offer 0.31 (SYNTH-LP-2)"));
    }

    #[test]
    fn countdown_is_live_then_expired() {
        let p = panel();
        // 5s before the deadline: a 5.000s countdown.
        let live = format_panel(&p, 1_000_000_000);
        assert!(live.contains("last_look 5.000s"), "live window:\n{live}");
        // Past the deadline: the row is no longer liftable.
        let lapsed = format_panel(&p, 7_000_000_000);
        assert!(lapsed.contains("last_look expired"), "lapsed:\n{lapsed}");
    }

    #[test]
    fn accept_side_maps_one_for_one() {
        assert_eq!(Side::from(CliAcceptSide::Buy), Side::Buy);
        assert_eq!(Side::from(CliAcceptSide::Sell), Side::Sell);
    }
}
