//! [`FixLpAdapter`] — an external liquidity provider on the panel, reached over
//! a **real FIX 4.4 session on a TCP socket**.
//!
//! This is a genuine [`celnet_fix::initiator::Initiator`] driving a real
//! `Logon → QuoteRequest(R) → Quote(S)` cycle against the LP's acceptor over a
//! loopback (or, at deploy, WAN) socket — NOT a mock. Per request the adapter:
//! 1. connects a fresh TCP stream to the LP's configured dial address;
//! 2. logs on and sends a FIX `QuoteRequest(R)` whose instrument block is
//!    translated from the Celnet [`RfqRequest`] via the `celnet-fix` dialect
//!    tags (`Symbol(55)`, `Product(460)`, `SecurityType(167)`, `PutOrCall(201)`,
//!    `StrikePrice(202)`, `StrikeCurrency(947)`);
//! 3. observes the returned `Quote(S)` (indicative — `LiftPolicy::Observe`, no
//!    lift: the panel only needs the two-way; the actual lift is the
//!    coordinator's `AcceptQuote` against the panel winner);
//! 4. translates the FIX `BidPx(132)` / `OfferPx(133)` (preferring the
//!    round-trip-exact dialect tags when present) back into a Celnet
//!    [`TwoWay`](crate::panel::TwoWay).
//!
//! Any transport / protocol failure, or no `Quote` within the deadline, maps to
//! [`QuoteSourceReply::NoQuote`] — the adapter is *dropped* from the panel, never
//! an error (a flaky LP must not fail the whole RFQ). The hard per-request
//! deadline is also enforced by the engine, so a wedged LP can never hang the
//! panel.
//!
//! # Honest boundary (verbatim — deploy/ENV-gated)
//!
//! **Live LP-panel connectivity (real bank sessions over WAN FIX) and the
//! regulated-venue / MAS-RMO status are ENV — designed, seamed and ADR'd
//! in-repo, validated at deploy, NEVER claimed in-repo.** In-repo this adapter
//! proves the aggregation/ranking/tie-break/last-look ALGORITHM and the FIX
//! framing/dialect round-trip over a **loopback** socket only. Live endpoints are
//! selected by the `CELNET_LP_PANEL` environment configuration on the edge,
//! defaulting to the synthetic in-repo panel; this crate carries no live
//! endpoint and makes no WAN-latency or venue-status claim.

use std::time::Duration;

use celnet_fix::dialect_rates::{
    self, BondQuoteRequestParams, RatesQuoteRequestParams, RatesSide, SubscriptionRequest,
};
use celnet_fix::framing::FrameEncoder;
use celnet_fix::initiator::{Initiator, LiftPolicy};
use celnet_fix::messages::Header;
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig};
use celnet_proto::{AccrualBasis, BrokenDate, PaymentFrequency, Side, rates_instrument};
use celnet_types::{Ccy, OptionType};
use tokio::net::TcpStream;

use crate::panel::{
    FxOptionLeg, QuoteSource, QuoteSourceReply, RatesLeg, RfqLeg, RfqRequest, TwoWay,
};

/// Static configuration for one external FIX LP: how to address its session and
/// what audit identity / validity window to stamp on its quotes.
#[derive(Debug, Clone)]
pub struct FixLpConfig {
    /// Stable audit `lp_id` for this LP (the panel tie-break key).
    pub lp_id: String,
    /// The LP acceptor's dial address (`host:port`). On loopback this is the
    /// ephemeral `127.0.0.1:<port>` the synthetic LP bound; at deploy it is the
    /// bank's FIX endpoint (ENV-provided, see the honest boundary).
    pub dial_addr: String,
    /// Our `SenderCompID` (the taker identity).
    pub sender_comp_id: Vec<u8>,
    /// The LP acceptor's `TargetCompID` (the venue identity).
    pub target_comp_id: Vec<u8>,
    /// The `SendingTime(52)` bytes used for the session frames (the edge's clock
    /// timestamp).
    pub sending_time: Vec<u8>,
    /// The logical timestamp (engine nanos clock) stamped on the resulting
    /// [`QuoteSourceReply`] as `epoch_nanos` (the panel tie-break key 1). The FIX
    /// `Quote`'s own `ValidUntilTime(62)` string is in wall time; the panel's
    /// last-look operates in the engine's nanos clock, so the adapter declares
    /// the window in that clock here.
    pub epoch_nanos: u64,
    /// How long (engine nanos) the LP's quote stays liftable from `epoch_nanos`;
    /// `valid_until_nanos = epoch_nanos + valid_for_nanos`.
    pub valid_for_nanos: u64,
}

/// An external LP reached over a real FIX session. Cheap to clone (config only);
/// each [`QuoteSource::request`] opens a fresh session.
#[derive(Debug, Clone)]
pub struct FixLpAdapter {
    cfg: FixLpConfig,
}

impl FixLpAdapter {
    /// Build an adapter from its config.
    #[must_use]
    pub fn new(cfg: FixLpConfig) -> Self {
        Self { cfg }
    }

    /// Run one real FIX `QuoteRequest → Quote` cycle and translate the result.
    /// `Ok(None)` means a clean no-quote (declined / no `Quote` returned, or a leg
    /// the FIX dialect does not encode); `Err` is a transport/protocol failure —
    /// both map to [`QuoteSourceReply::NoQuote`] at the call site (an LP is
    /// dropped, never an error).
    ///
    /// The instrument block is built from the [`RfqLeg`] via the class-appropriate
    /// `celnet-fix` dialect: an FX-option leg through the FX vanilla tags, a
    /// fixed-income leg through the rates dialect (`SecurityType(167)=OIS` /
    /// `BOND`). The `Quote(S)` reply is parsed identically for both — the panel
    /// ranks the same `TwoWay`, so a rates LP and an FX LP share one seam.
    async fn run_cycle(&self, request: &RfqRequest) -> std::io::Result<Option<TwoWay>> {
        let stream = TcpStream::connect(&self.cfg.dial_addr).await?;
        let cfg = SessionConfig {
            sender: self.cfg.sender_comp_id.clone(),
            target: self.cfg.target_comp_id.clone(),
            heart_bt_int: 30,
            role: Role::Initiator,
        };
        let mut init = Initiator::new(Session::new(cfg, InMemoryStore::new()), LiftPolicy::Observe);

        let req_id = request.request_id.clone().into_bytes();
        let sending_time = self.cfg.sending_time.clone();

        // Build the FIX instrument block from the class-specific leg. An
        // unsupported / malformed rates leg (an IRS/FRA the OIS+bond dialect does
        // not encode, or a bad enum) is a clean no-quote — the LP is simply absent.
        let result = match &request.leg {
            RfqLeg::FxOption(fx) => {
                let symbol = fx_symbol(fx);
                let strike = fx.strike;
                let put_or_call: &[u8] = match fx.option_type {
                    OptionType::Call => b"1",
                    OptionType::Put => b"0",
                };
                let strike_ccy = ccy_bytes(fx.pair.quote);
                let build_req = move |h: &Header<'_>, e: &mut FrameEncoder| {
                    e.clear();
                    h.encode(celnet_fix::MsgType::QuoteRequest, e);
                    e.push(131, &req_id);
                    e.push(55, &symbol);
                    e.push(460, b"4"); // Product = CURRENCY
                    e.push(167, b"FXVO"); // deliverable FX vanilla option
                    e.push(201, put_or_call);
                    push_strike(e, strike);
                    e.push(947, &strike_ccy);
                    e.push(1194, b"0"); // European exercise
                    e.finish()
                };
                init.request_and_lift(stream, sending_time, build_req)
                    .await?
            }
            RfqLeg::Rates(rates) => {
                let Some(spec) = rates_fix_spec(rates) else {
                    return Ok(None); // a leg the FIX rates dialect does not encode.
                };
                let symbol = rates.symbol.clone();
                let build_req = move |h: &Header<'_>, e: &mut FrameEncoder| {
                    // Reuse the `celnet-fix` rates dialect builders verbatim — the
                    // SAME wire an inbound FI RFQ decodes on the acceptor side.
                    match &spec {
                        RatesFixSpec::Ois {
                            tenor_years,
                            side,
                            notional,
                        } => dialect_rates::build_rates_quote_request(
                            h,
                            &RatesQuoteRequestParams {
                                quote_req_id: &req_id,
                                symbol: &symbol,
                                tenor_years: *tenor_years,
                                notional: *notional,
                                side: *side,
                                subscription: SubscriptionRequest::Snapshot,
                            },
                            e,
                        ),
                        RatesFixSpec::Bond {
                            coupon_rate,
                            coupon_frequency,
                            day_count,
                            maturity,
                            redemption,
                            notional,
                            side,
                        } => dialect_rates::build_bond_quote_request(
                            h,
                            &BondQuoteRequestParams {
                                quote_req_id: &req_id,
                                symbol: &symbol,
                                coupon_rate: *coupon_rate,
                                coupon_frequency: *coupon_frequency,
                                day_count: *day_count,
                                maturity: *maturity,
                                redemption: *redemption,
                                notional: *notional,
                                side: *side,
                                subscription: SubscriptionRequest::Snapshot,
                            },
                            e,
                        ),
                    }
                };
                init.request_and_lift(stream, sending_time, build_req)
                    .await?
            }
        };

        // `InitiatorResult` surfaces the standard parsed bid/offer, exact to wire
        // precision and what the panel ranks on — identical for FX and rates.
        match (result.bid, result.offer) {
            (Some(bid), Some(offer)) => Ok(Some(TwoWay { bid, offer })),
            _ => Ok(None),
        }
    }
}

/// The rates-dialect FIX instrument block for a [`RatesLeg`], pre-validated so the
/// builder closure is infallible. `None` for a leg the OIS+bond dialect does not
/// encode (an IRS/FRA arm, an unset arm, or a malformed bond enum) — a clean
/// no-quote, never a partial/faked request.
enum RatesFixSpec {
    /// An OIS `QuoteRequest(R)` (`SecurityType(167)=OIS`).
    Ois {
        tenor_years: u32,
        side: RatesSide,
        notional: f64,
    },
    /// A cash-bond `QuoteRequest(R)` (`SecurityType(167)=BOND`).
    Bond {
        coupon_rate: f64,
        coupon_frequency: PaymentFrequency,
        day_count: AccrualBasis,
        maturity: BrokenDate,
        redemption: f64,
        notional: f64,
        side: Side,
    },
}

/// Extract the FIX rates-dialect request spec from a [`RatesLeg`], mapping the
/// wire instrument arm onto the OIS or cash-bond `QuoteRequest(R)` the
/// `celnet-fix` rates dialect encodes. Returns `None` for an arm the dialect does
/// not carry (vanilla IRS / FRA — the dialect is OIS + bond only), an unset arm,
/// or a bond with a malformed enum / missing maturity (all clean no-quotes).
fn rates_fix_spec(rates: &RatesLeg) -> Option<RatesFixSpec> {
    match rates.instrument.instrument.as_ref()? {
        rates_instrument::Instrument::Ois(ois) => Some(RatesFixSpec::Ois {
            tenor_years: ois.tenor_years,
            side: proto_side_to_rates(rates.side),
            notional: rates.notional,
        }),
        rates_instrument::Instrument::Bond(bond) => Some(RatesFixSpec::Bond {
            coupon_rate: bond.coupon_rate,
            coupon_frequency: PaymentFrequency::try_from(bond.coupon_frequency).ok()?,
            day_count: AccrualBasis::try_from(bond.day_count).ok()?,
            maturity: bond.maturity_date?,
            redemption: bond.redemption,
            notional: rates.notional,
            side: rates.side,
        }),
        // The FIX rates dialect encodes OIS + cash bond only; a vanilla IRS / FRA
        // leg is honestly a no-quote on the FIX LP route (the native in-process
        // source still prices it).
        rates_instrument::Instrument::Irs(_) | rates_instrument::Instrument::Fra(_) => None,
    }
}

/// Map a wire [`Side`] to the rates dialect [`RatesSide`]: pay-fixed is a
/// (fixed-rate) buy, receive-fixed a sell, a two-way request carries no firm side.
fn proto_side_to_rates(side: Side) -> RatesSide {
    match side {
        Side::Buy => RatesSide::PayFixed,
        Side::Sell => RatesSide::ReceiveFixed,
        Side::TwoWay => RatesSide::TwoWay,
    }
}

impl QuoteSource for FixLpAdapter {
    fn lp_id(&self) -> &str {
        &self.cfg.lp_id
    }

    fn request<'a>(
        &'a self,
        request: &'a RfqRequest,
        _deadline: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = QuoteSourceReply> + Send + 'a>> {
        Box::pin(async move {
            match self.run_cycle(request).await {
                Ok(Some(price)) => QuoteSourceReply::Quote {
                    price,
                    epoch_nanos: self.cfg.epoch_nanos,
                    valid_until_nanos: self
                        .cfg
                        .epoch_nanos
                        .saturating_add(self.cfg.valid_for_nanos),
                },
                // A clean no-quote OR any transport/protocol error: the LP is
                // simply absent from the panel (dropped, not errored).
                Ok(None) | Err(_) => QuoteSourceReply::NoQuote,
            }
        })
    }
}

/// The 6-byte `Symbol(55)` for an FX-option leg's pair (e.g. `b"EURUSD"`).
fn fx_symbol(fx: &FxOptionLeg) -> Vec<u8> {
    let mut s = Vec::with_capacity(6);
    s.extend_from_slice(fx.pair.base.as_str().as_bytes());
    s.extend_from_slice(fx.pair.quote.as_str().as_bytes());
    s
}

/// A currency's 3 ASCII bytes.
fn ccy_bytes(ccy: Ccy) -> Vec<u8> {
    ccy.as_str().as_bytes().to_vec()
}

/// Push a `StrikePrice(202)` with up to 8-dp fixed precision (mirrors the
/// `celnet-fix` test framing so the acceptor decodes the identical strike).
fn push_strike(e: &mut FrameEncoder, strike: f64) {
    let scaled = (strike * 1e8).round() as u128;
    let int = scaled / 100_000_000;
    let frac = scaled % 100_000_000;
    let s = format!("{int}.{frac:08}");
    e.push(202, s.as_bytes());
}
