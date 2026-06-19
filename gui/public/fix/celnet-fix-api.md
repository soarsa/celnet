# celnet FIX API — FX options quote venue (FIX 4.4)

**Audience:** an engineer (or an AI agent) building a FIX client that connects to a
celnet inbound acceptor to request quotes and lift them. This is the canonical,
human- and agent-readable spec; the machine-readable contract is the companion
QuickFIX data dictionary [`celnet-fix44.xml`](./celnet-fix44.xml). Both are
served from the running GUI under `/fix/` and downloadable from the **Connections**
workspace.

It is the projection of the engine's own dialect — `dictionary.rs` (the tag/
required-field table), `dialect_fx.rs` (the instrument + strategy mapping and
convention checks), and `messages.rs` (the wire builders) — so there is one
contract and no drift.

---

## 1. What it is

Each managed connection in the **Connections** workspace binds a **FIX 4.4
acceptor** on a `host:port`. It is a **quote venue** for FX options: a connected
counterparty sends an RFQ, receives a firm two-way quote, and may lift it; the
venue replies with an execution report.

The application lifecycle is:

```
client  --  QuoteRequest (R)      -->  venue       request a price (RFQ)
client  <-- Quote (S)             --   venue       firm two-way + QuoteID
client  --  NewOrderSingle (D)    -->  venue       lift by QuoteID + Side
client  <-- ExecutionReport (8)   --   venue       fill (F) or reject (8)
```

Everything below the application layer is standard FIX 4.4 session behaviour
(logon, heartbeats, test requests, resend/gap-fill, logout).

---

## 2. Connecting

A connection's row in the Connections workspace shows its **Bind address**,
**SenderCompID**, **TargetCompID**, and owning **Desk**. To connect as a client:

| Your client setting | Value to use |
|---|---|
| `BeginString` | `FIX.4.4` |
| `SocketConnectHost` / `SocketConnectPort` | the connection's **Bind address** |
| `SenderCompID` (yours) | the connection's **TargetCompID** |
| `TargetCompID` (theirs) | the connection's **SenderCompID** |
| `EncryptMethod(98)` | `0` (none) |
| `HeartBtInt(108)` | your interval in seconds, e.g. `30` |

> **CompID rule (critical).** The acceptor authenticates the peer by CompID: it
> requires every inbound message to carry `SenderCompID(49) = <its TargetCompID>`
> and `TargetCompID(56) = <its SenderCompID>`. In other words **you swap the two
> CompIDs shown on the row** — your `SenderCompID` is the venue's `TargetCompID`,
> and vice-versa. A mismatch is refused at the session layer.

The **Client config** button on each connection row downloads a ready-to-use
QuickFIX initiator `.cfg` with these values already filled in (CompIDs swapped,
host/port set, pointing at this dictionary).

### Logon

Send `Logon(A)` with `EncryptMethod(98)=0` and your `HeartBtInt(108)`. Set
`ResetSeqNumFlag(141)=Y` on the first connect to start both sequence streams at 1.
The venue replies with its own `Logon(A)` and heartbeats flow.

---

## 3. Requesting a quote — `QuoteRequest (R)`

### 3.1 Single-leg vanilla

A single-leg RFQ carries a **flat instrument block** (no repeating group):

| Tag | Field | Required | Notes |
|---|---|:---:|---|
| 131 | QuoteReqID | **Y** | your RFQ correlation id; echoed on the Quote |
| 55 | Symbol | Y* | 6-letter pair `EURUSD` (or `EUR/USD`) |
| 460 | Product | — | `4` (CURRENCY) if present |
| 167 | SecurityType | Y* | `FXVO` deliverable · `FXNO` non-deliverable |
| 201 | PutOrCall | Y* | `0` put · `1` call |
| 202 | StrikePrice | Y* | quote ccy per base, strictly positive |
| 947 | StrikeCurrency | — | must equal the pair's **quote** ccy (e.g. `USD` for EURUSD); defaults to it if omitted |
| 1194 | ExerciseStyle | — | `0` European (default) · `1` American |
| 541 | MaturityDate | — | `YYYYMMDD`; the edge resolves the tenor from it |
| **7001** | **ExpiryYears** | Y* | **custom**: exact vol-time in years (e.g. `0.25`) |

`Y*` = required by the dialect mapper (not by the FIX dictionary's required set);
omitting one yields a specific reject reason (§6).

> **Why ExpiryYears(7001)?** Pricing is vol-time based. Carrying the exact
> `expiry_years` on a user-defined tag makes the RFQ fully wire-specified, so the
> returned premium reproduces the engine/golden price to the bit, independent of
> any calendar date. FIX tolerates unknown tags; this is a dialect provenance
> field, never a vendor name.

**Convention checks** (enforced on every RFQ, because *convention error dwarfs
model error*):
- `StrikeCurrency(947)` must be the pair's quote currency.
- `SecurityType(167)` (deliverable vs non-deliverable) must agree with the
  resolved convention for `(pair, tenor)`.

### 3.2 Strategy (multi-leg)

A strategy RFQ replaces the flat block with a `NoLegs(555)` repeating group; each
leg starts at `LegSymbol(600)`:

| Tag | Field | Notes |
|---|---|---|
| 555 | NoLegs | leg count (> 0) |
| 600 | LegSymbol | starts a leg; the pair |
| 609 | LegSecurityType | `FXVO`/`FXNO` (defaults deliverable) |
| 1358 | LegPutOrCall | `0` put · `1` call |
| 612 | LegStrikePrice | strictly positive |
| 624 | LegSide | `1` buy · `2` sell |
| 623 | LegRatioQty | positive ratio (default `1`) |

The package premium is the side- and ratio-weighted sum of the per-leg
Garman–Kohlhagen premia off **one** market snapshot (so straddle/risk-reversal/
fly reconcile exactly).

---

## 4. The quote — `Quote (S)`

| Tag | Field | Notes |
|---|---|---|
| 131 | QuoteReqID | echoed from your RFQ |
| 117 | QuoteID | **the last-look / idempotency anchor — quote this back to trade** |
| 55 | Symbol | the pair |
| 132 / 133 | BidPx / OfferPx | two-way premium (pip-resolution, 8 dp) |
| 134 / 135 | BidSize / OfferSize | quote size |
| 62 | ValidUntilTime | quote expiry (UTC) |
| **7011 / 7012** | **BidPxExact / OfferPxExact** | the two-way to full f64 precision (provenance; reconciles a fill to the engine price to the last bit) |

---

## 5. Lifting the quote — `NewOrderSingle (D)` → `ExecutionReport (8)`

To trade, send `NewOrderSingle(D)`:

| Tag | Field | Required | Notes |
|---|---|:---:|---|
| 11 | ClOrdID | **Y** | your order id (echoed back) |
| 117 | QuoteID | Y* | the `QuoteID` from the Quote you are lifting |
| 55 | Symbol | — | the pair |
| 54 | Side | **Y** | `1` buy (lift the offer) · `2` sell (hit the bid) |
| 38 | OrderQty | **Y** | quantity |
| 40 | OrdType | — | `D` previously-quoted |
| 60 | TransactTime | — | UTC |

The venue replies with `ExecutionReport(8)`:

| Tag | Field | Notes |
|---|---|---|
| 37 | OrderID | venue-minted |
| 17 | ExecID | execution id |
| 11 | ClOrdID | echoed |
| 150 | ExecType | `F` trade (fill) · `8` rejected |
| 39 | OrdStatus | `F` filled · `8` rejected |
| 54 | Side | as ordered |
| 32 / 31 | LastQty / LastPx | fill quantity / locked premium |
| **7013** | **LastPxExact** | fill premium to full f64 precision (provenance) |
| 442 | MultiLegReportingType | for package fills |
| 58 | Text | rejection reason, when `ExecType=8` |

> **Last-look.** A `QuoteID` is single-use. Lifting a stale, already-consumed,
> forged, or unknown `QuoteID` returns `ExecutionReport(ExecType=8, OrdStatus=8)`
> with a `Text(58)` reason — never a fill. Re-RFQ to get a fresh `QuoteID`.

---

## 6. Rejects

- **Session layer** — malformed framing, checksum/body-length errors, CompID
  mismatch, or sequence problems are handled by FIX session rules (`Reject(3)`,
  `ResendRequest(2)`, `SequenceReset(4)`).
- **Dictionary** — an unknown `MsgType`, a missing dictionary-required tag, a
  `PossDupFlag(43)=Y` resend without `OrigSendingTime(122)`, or a field whose
  value fails its scalar type is rejected before mapping.
- **Dialect / convention** — a bad symbol, non-`CURRENCY` product, bad security
  type, missing/zero strike, strike-currency ≠ quote, wrong exercise byte, or a
  settlement style disagreeing with the resolved convention is rejected with a
  specific reason. Unknown tags are tolerated (the dialect maps only what it
  understands).

---

## 7. Worked example (SOH shown as `|`)

```
# Logon
8=FIX.4.4|35=A|49=CELNET-CPTY|56=CELNET|34=1|52=20260619-09:00:00.000|98=0|108=30|141=Y|10=...

# RFQ: 3M EUR/USD 1.0950 call, deliverable, European, vol-time 0.25y
8=FIX.4.4|35=R|49=CELNET-CPTY|56=CELNET|34=2|52=...|131=RFQ-1|55=EURUSD|460=4|167=FXVO|201=1|202=1.0950|947=USD|1194=0|7001=0.25|10=...

# Quote back
8=FIX.4.4|35=S|49=CELNET|56=CELNET-CPTY|34=2|52=...|131=RFQ-1|117=Q-1001|55=EURUSD|132=0.01180000|133=0.01220000|134=1000000.00000000|135=1000000.00000000|62=20260619-09:00:30.000|7011=0.0118...|7012=0.0122...|10=...

# Lift the offer (buy)
8=FIX.4.4|35=D|49=CELNET-CPTY|56=CELNET|34=3|52=...|11=ORD-1|117=Q-1001|55=EURUSD|54=1|38=1000000|40=D|60=...|10=...

# Fill
8=FIX.4.4|35=8|49=CELNET|56=CELNET-CPTY|34=3|52=...|37=O-5001|17=E-7001|11=ORD-1|150=F|39=F|55=EURUSD|54=1|32=1000000.00000000|31=0.01220000|7013=0.0122...|10=...
```

---

## 8. Build a client

Any QuickFIX-family engine works. Point the session's `DataDictionary` at
[`celnet-fix44.xml`](./celnet-fix44.xml).

**quickfix-go** initiator settings (the **Client config** download generates this
for a specific connection, CompIDs already swapped):

```ini
[DEFAULT]
ConnectionType=initiator
ReconnectInterval=5
FileStorePath=store
FileLogPath=log

[SESSION]
BeginString=FIX.4.4
SenderCompID=CELNET-CPTY        # the connection's TargetCompID
TargetCompID=CELNET             # the connection's SenderCompID
SocketConnectHost=127.0.0.1
SocketConnectPort=9099          # the connection's Bind address
HeartBtInt=30
ResetOnLogon=Y
DataDictionary=celnet-fix44.xml
```

Then, after logon, send a `QuoteRequest(R)` per §3, read the `Quote(S)`, and
(optionally) lift it with a `NewOrderSingle(D)` per §5.

A reference Rust price-taker that drives this exact flow lives in the repo at
`crates/celnet-fix/examples/fix_rfq_client.rs`.
