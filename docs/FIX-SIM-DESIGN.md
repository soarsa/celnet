# FIX Client-Simulator Bot (`fix-sim`) — Design

- **Status:** Proposed design (2026-07-01). **Design only — no bot code yet.** A runnable
  UAT start script (`deploy/start-fix-sim.sh`) ships alongside, wrapping the existing
  `celnet-fix` example RFQ client until the full `fix-sim` binary is built.
- **Build home when scheduled:** a branch off **`main`** — a FIX client simulator is
  general estate tooling and the FXO acceptor is main's core. (Authored on
  `feature/fi-reference-data` as a design artifact.)
- **Reuses:** `crates/celnet-fix` — `Initiator` (client session), `acceptor.rs` (maker
  side), `dialect_fx.rs` / `dialect_rates.rs`, `examples/fix_rfq_client.rs`,
  `tests/session_loopback.rs`.
- **Adds (one acceptor-side piece):** `SecurityListRequest` / `SecurityList` (35=x / 35=y)
  to celnet-fix (chosen securities-download path).

## 1. Purpose

An external FIX **initiator** bot that exercises the live celnet FIX acceptor end-to-end the
way a real institutional client does: log on, **download the tradable securities**, send
**RFQs** on a timer, receive **quotes** (priced by the desk's human traders in the GUI, or
auto-quoted), then **randomly act** — accept-and-fill, cancel/pass, or let expire — and, when a
trader **rejects** the RFQ, **back off a random interval** and resume. It doubles as a
**soak / load / realism harness** for the whole RFQ->Quote->Order->Fill lifecycle and the FIX
session layer.

Vendor-neutral naming (guardrail #8): bot core identifiers are purpose-named. `CELER_FXO` /
`CELNET` are session CompIDs (Celer is our own estate — fine in session/config context).

## 2. What already exists (reuse, don't rebuild)

celnet-fix already implements **both sides** of the flow:
- **`Initiator`** — logon, heartbeat, seqnum/resend, `ReconnectInterval` reconnect-and-resume.
- **Client flow** (per `initiator.rs` / `examples/fix_rfq_client.rs`): send `QuoteRequest(R)`
  -> receive `Quote(S)` -> lift with `NewOrderSingle(D)` referencing `QuoteID` (last-look) ->
  collect `ExecutionReport(8)` (`EXEC_FILLED` / `EXEC_REJECTED`); `QuoteRequestReject(AG)` on
  trader decline.
- **Dialects:** `dialect_fx` (FXO desk) and `dialect_rates` (FI desk, incl.
  `MarketDataRequest` subscribe/unsubscribe for streaming rates).
- **Maker side:** RFQs route to the desk inbox; human traders price or reject in the GUI (or
  auto-quote). The bot only reacts to `Quote` vs `QuoteRequestReject`.

So `fix-sim` is an **orchestration + policy layer** over these primitives, plus the one
SecurityList addition.

## 3. The one new piece — `SecurityListRequest` / `SecurityList` (35=x / 35=y)

celnet-fix has no security-download message today. Add it acceptor-side so the bot downloads
the universe over FIX like a real client (chosen over a config seed or an out-of-band registry
call — most faithful).

- **Inbound `SecurityListRequest(35=x)`:** `SecurityReqID(320)`,
  `SecurityListRequestType(559)` (0=Symbol, 4=all securities). Optionally scoped by
  `Product(460)` / desk.
- **Outbound `SecurityList(35=y)`:** `SecurityReqID(320)`, `SecurityRequestResult(560)`,
  `TotNoRelatedSym(393)`, then a `NoRelatedSym(146)` repeating group of `Symbol(55)` (+
  `SecurityID`/`SecurityIDSource`, `SecurityType`, `Currency`, tenor/leg metadata per dialect).
- **Source of truth (acceptor side):** map from the desk's authoritative universe —
  **FX pairs** (surface/pairs) for the FXO dialect, **reference-data registry**
  (`IdentityStore` / `ListInstruments`) for the rates dialect. A thin FIX projection of data
  that already exists; no new store.
- Large lists paginate via multiple `SecurityList` fragments (`LastFragment(893)`).

## 4. Bot architecture

```
 config (session block + policy) --> fix-sim
        |
        v
 +---------------------------------------------------------------+
 |  celnet_fix::Initiator  (logon, heartbeat, seqnum, reconnect) |
 +---------------+-----------------------------------------------+
                 | on logon
                 v
    SecurityListRequest(x) --> SecurityList(y) --> universe: [symbols]
                 |
                 v
    scheduler: spawn a per-security AGENT (bounded in-flight concurrency)
                 |
                 v
    +------------ per-security agent state machine ------------+
    | IDLE -(timer N +/- jitter)-> QuoteRequest(R)             |
    | AWAIT_QUOTE                                              |
    |   |- Quote(S) -> DECIDE -+ accept(p_a) -> NewOrderSingle |
    |   |                      |        `-> AWAIT_EXEC -> Exec |
    |   |                      +- cancel(p_c) -> QuoteCancel   |
    |   |                      `- expire ----------> IDLE(jit) |
    |   `- QuoteRequestReject(AG) -> BACKOFF(rand T) -> IDLE   |
    +---------------------------------------------------------+
                 |
                 v
    (optional) MarketDataRequest(V) subscribe -> stream snapshots (log/telemetry)
```

- **Policy engine (seeded RNG for reproducibility):** RFQ cadence (mean + jitter, "every few
  minutes"), accept / cancel / expire probabilities, order qty & side distributions, in-flight
  concurrency, and the reject-backoff distribution. All config-driven.
- **Dialect-parametrized:** one bot instance targets one desk (FX or rates) via the matching
  celnet-fix dialect.
- **Streaming rates (optional):** subscribe via `MarketDataRequest`; snapshots feed telemetry
  and can bias RFQ aggression (read-only for v1).
- **Observability:** structured lifecycle log + counters — RFQs/min, fill ratio, reject ratio,
  quote-latency HdrHistogram — via `celnet-observability`. Makes it a soak/load harness.
- **Resilience:** `ReconnectInterval=5` reconnect-and-resume via the Initiator; in-flight
  agents reconcile against resent `ExecutionReport`s on reconnect.
- **Multi-client:** run N identities (distinct SenderCompIDs) for a realistic client mix.

## 5. Config (extends the celnet-generated QuickFIX client profile)

```toml
[session]                 # mirrors the celnet-generated client config
begin_string    = "FIX.4.4"
sender_comp_id  = "CELER_FXO"
target_comp_id  = "CELNET"
connect_host    = "127.0.0.1"
connect_port    = 56001
heartbeat_secs  = 30
reconnect_secs  = 5
data_dictionary = "celnet-fix44.xml"
dialect         = "fx"    # fx | rates

[policy]
rfq_period_secs     = 180  # "every few minutes"
rfq_jitter_secs     = 60
accept_prob         = 0.5
cancel_prob         = 0.3  # remainder = let-expire
max_in_flight       = 8
reject_backoff_secs = { min = 120, max = 600 }
qty                 = { min = 1_000_000, max = 25_000_000 }
seed                = 42   # reproducible runs
subscribe_market_data = false
```

## 6. Build order (each loopback-gated)

1. **`SecurityListRequest`/`SecurityList` in celnet-fix** (+ acceptor projection from
   pairs/registry); loopback test round-trips the universe.
2. **`fix-sim` skeleton** — config + `Initiator` logon + SecurityList download; logs the universe.
3. **Per-security RFQ->Quote->decide->order->fill agent** with the seeded policy engine.
4. **Reject-backoff + reconnect-resume** paths.
5. **Streaming-rates subscription + observability counters/histogram.**
6. **Multi-identity client mix + a soak profile.**

## 7. UAT operation — `deploy/start-fix-sim.sh`

The start script (shipped now) launches the simulator on the UAT box:
- sources the Rust env, creates `store/` + `log/` dirs (mirrors the QuickFIX
  `FileStorePath`/`FileLogPath`),
- TCP-preflights the acceptor `host:port` before launching,
- prefers the `fix-sim` binary when built; **falls back to the runnable
  `cargo run -p celnet-fix --example fix_rfq_client`** so UAT has a live client today,
- reads env overrides (`FIXSIM_HOST`, `FIXSIM_PORT`, `FIXSIM_SENDER`, `FIXSIM_TARGET`,
  `FIXSIM_DIALECT`, `FIXSIM_CONFIG`), backgrounds with a PID file, logs to `log/`.

See the script header for usage. It is intentionally honest: if neither the binary nor the
example is available it exits non-zero with a clear message rather than pretending to start.

## 8. Testing

- **Loopback (primary):** drive `fix-sim` against the in-tree celnet-fix acceptor
  (`tests/session_loopback.rs` precedent) — deterministic, seeded, no network.
- **Live UAT:** point at `127.0.0.1:56001` with the real acceptor + desk GUI; a human prices/
  rejects RFQs and the bot's counters reflect it.
- **No new commercial deps** (guardrail #7): our own Initiator, no external QuickFIX runtime.

## 9. Open questions

- **Auto-quote vs desk-only:** does the FXO acceptor auto-quote, or must a human price every
  RFQ? Affects unattended soak runs (if desk-only, add an optional auto-maker).
- **SecurityList scope:** all-securities vs per-desk/currency filtering — recommend
  `SecurityListRequestType=4` with an optional `Currency`/`Product` filter.
- **Last-look window:** confirm the acceptor's quote TTL so accept latency stays inside it.

## 10. References

- `crates/celnet-fix/` (`initiator.rs`, `acceptor.rs`, `dialect_fx.rs`, `dialect_rates.rs`,
  `examples/fix_rfq_client.rs`, `tests/session_loopback.rs`)
- `deploy/start-fix-sim.sh`, `docs/CELER-INTEGRATION.md`, `docs/W4-STRUCTURED-RFQ-PLAN.md`
- FIX 4.4: `QuoteRequest(R)`, `Quote(S)`, `QuoteRequestReject(AG)`, `NewOrderSingle(D)`,
  `ExecutionReport(8)`, `SecurityListRequest(x)`, `SecurityList(y)`, `MarketDataRequest(V)`.
