# FIX API — inbound acceptor client integration

This is the index entry in the design corpus for the **external FIX client API**:
the contract a counterparty uses to connect into a managed inbound FIX acceptor
(defined in the GUI **Connections** workspace) and trade FX options via
RFQ → Quote → NewOrderSingle → ExecutionReport.

## Artifacts (downloadable from the GUI)

The full spec lives as two artifacts served by the GUI under `/fix/` and
downloadable from the Connections workspace toolbar:

- **FIX API guide** — [`gui/public/fix/celnet-fix-api.md`](../../gui/public/fix/celnet-fix-api.md):
  the human/agent-readable spec (session layer, the CompID-swap rule, the RFQ→
  Quote→lift→exec flow, single-leg + multileg instrument blocks, convention
  checks, reject semantics, a worked wire transcript, and a "build a client"
  quickstart). This is the **claude spec** — point an agent at it to generate a client.
- **QuickFIX data dictionary** — [`gui/public/fix/celnet-fix44.xml`](../../gui/public/fix/celnet-fix44.xml):
  the machine-readable FIX 4.4 dialect (MsgTypes, fields, required sets, enums,
  custom tags) to drop into any QuickFIX-family engine as the session
  `DataDictionary`. This is the **fix document**.

Per-connection, the Connections workspace also generates a ready-to-use QuickFIX
**initiator `.cfg`** (CompIDs swapped, host/port filled in) via the row's
**Client config** button — see `gui/src/lib/fixClientConfig.ts`.

## Source of truth (no drift)

Both artifacts are the projection of the engine's own dialect, which remains the
authority:

- `crates/celnet-fix/src/dictionary.rs` — the tag table (`TAGS`), `MsgType`s, and
  `required_tags` per message.
- `crates/celnet-fix/src/dialect_fx.rs` — the instrument/strategy mapping, the
  custom provenance tags (`TAG_EXPIRY_YEARS` 7001), and the convention checks.
- `crates/celnet-fix/src/messages.rs` — the wire builders (`build_quote`,
  `build_execution_report`, …) and the exact-premium tags (7011/7012/7013).
- `crates/celnet-fix/src/session.rs` — the FIX 4.4 session layer and the CompID
  authentication rule.
- `crates/celnet-fix/examples/fix_rfq_client.rs` — a reference Rust price-taker
  driving the exact flow.

When the dialect changes, update the two `gui/public/fix/` artifacts in the same
change (and re-run `gui/test/fixClientConfig.test.ts`). See also
`docs/CELER-FIX-INTEGRATION-PLAN.md` for the §1.2 tag map and integration plan.
