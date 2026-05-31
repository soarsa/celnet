/**
 * The ticket task-pane controller: binds the pure ticket state machines
 * (ticketModel.ts) to the DOM and the shared WS connection (one contract).
 *
 * It does NOT recompute prices — every number comes from the server over the WS
 * mirror, so the task-pane two-way is bit-identical to the cell, the GUI, and the
 * SDK. RFQ requests a two-way and books a directional trade; the Contribute panel
 * stages a mark (idempotent) and commits it on explicit confirmation (never on a
 * recalc). Connection state drives the header pill.
 */

import { Connection } from "../transport/connection";
import { browserWebSocketFactory } from "../transport/socket";
import { DEFAULT_CONVENTIONS, shapeVanillaInstrument } from "../functions/shaping";
import { getStaged, stageMark, markCommitted, markRejected } from "../functions/markStaging";
import { brokerQuoteSetToWire, ccyPairToWire, conventionsToWire } from "../contract/wsCodec";
import { smileModel } from "../contract/enums";
import {
  IDLE_MARK,
  IDLE_RFQ,
  canCommitMark,
  markCommittedState,
  markRejectedState,
  markStaged,
  rfqExecuted,
  rfqQuoted,
  rfqRejected,
  type MarkState,
  type RfqState,
} from "./ticketModel";

const DEFAULT_ENDPOINT = "ws://127.0.0.1:8081";

function endpoint(): string {
  const g = globalThis as unknown as { CELNET_WS_ENDPOINT?: string };
  return typeof g.CELNET_WS_ENDPOINT === "string" && g.CELNET_WS_ENDPOINT.length > 0
    ? g.CELNET_WS_ENDPOINT
    : DEFAULT_ENDPOINT;
}

function el<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing element #${id}`);
  return node as T;
}

function val(id: string): string {
  return el<HTMLInputElement>(id).value.trim();
}

function setState(id: string, text: string, tone: "" | "ok" | "warn" | "bad" = ""): void {
  const node = el(id);
  node.textContent = text;
  node.className = `state${tone ? " " + tone : ""}`;
}

function boot(): void {
  const conn = new Connection({ url: endpoint(), factory: browserWebSocketFactory() });
  const connPill = el("conn");
  conn.onState((open: boolean) => {
    connPill.textContent = open ? "connected" : "disconnected";
    connPill.className = `pill ${open ? "pill-on" : "pill-off"}`;
  });

  let rfq: RfqState = IDLE_RFQ;
  let mark: MarkState = IDLE_MARK;

  const renderRfq = (): void => {
    el("rfq-result").textContent =
      rfq.phase === "idle" ? "" : `${rfq.status}\nvalid until ${formatNanos(rfq.validUntilNanos)}`;
    (el("trade-buy") as HTMLButtonElement).disabled = !rfq.tradable;
    (el("trade-sell") as HTMLButtonElement).disabled = !rfq.tradable;
  };

  el("rfq-go").addEventListener("click", () => {
    void (async () => {
      try {
        setState("rfq-state", "requesting…", "warn");
        const instrument = shapeVanillaInstrument({
          pair: val("rfq-pair"),
          tenor: val("rfq-tenor"),
          strikeOrDelta: val("rfq-strike"),
          callPut: val("rfq-cp"),
          notional: Number(val("rfq-notional")),
        });
        const quote = await conn.requestQuote(
          instrument,
          DEFAULT_CONVENTIONS,
          `taskpane-rfq:${Date.now()}`,
        );
        rfq = rfqQuoted({
          bid: quote.price.bid,
          offer: quote.price.offer,
          quoteId: quote.quoteId,
          validUntilNanos: quote.validUntilNanos,
        });
        setState("rfq-state", "quoted", "ok");
        renderRfq();
      } catch (err) {
        setState("rfq-state", message(err), "bad");
      }
    })();
  });

  const trade = (side: "BUY" | "SELL"): void => {
    void (async () => {
      if (rfq.phase !== "pending") return;
      try {
        setState("trade-state", `accepting ${side}…`, "warn");
        const reply = await conn.request(
          "accept_quote",
          { quote_id: Number(rfq.quoteId), idempotency_key: `taskpane-exec:${rfq.quoteId}`, side: side === "BUY" ? 0 : 1 },
          "execution",
        );
        const premium = typeof reply["traded_premium"] === "number" ? reply["traded_premium"] : 0;
        rfq = rfqExecuted(rfq, side, premium);
        setState("trade-state", "executed", "ok");
        renderRfq();
      } catch (err) {
        rfq = rfqRejected(rfq, message(err));
        setState("trade-state", message(err), "bad");
        renderRfq();
      }
    })();
  };
  el("trade-buy").addEventListener("click", () => trade("BUY"));
  el("trade-sell").addEventListener("click", () => trade("SELL"));

  el("mk-stage").addEventListener("click", () => {
    void (async () => {
      try {
        const status = await stageMark(conn, {
          pair: val("mk-pair"),
          tenor: val("mk-tenor"),
          pillar: val("mk-pillar"),
          vol: Number(val("mk-vol")),
          comment: val("mk-comment"),
          model: val("mk-model"),
        });
        mark = markStaged(status.stagingId);
        (el("mk-commit") as HTMLButtonElement).disabled = !canCommitMark(mark);
        setState("mk-state", "staged — confirm to contribute", "warn");
        el("mk-pending").textContent = `PENDING ${status.stagingId}`;
      } catch (err) {
        setState("mk-state", message(err), "bad");
      }
    })();
  });

  el("mk-commit").addEventListener("click", () => {
    void (async () => {
      if (!canCommitMark(mark)) return;
      try {
        setState("mk-state", "contributing…", "warn");
        // The commit: a single broker-quote-set mark for the (pair, tenor), with
        // the declared convention checked server-side (normalize cross-check). A
        // material mismatch is rejected, never silently corrupting the surface.
        // Carry the model the trader selected at stage time into the commit, so
        // the surface is calibrated with the chosen family (VV/SABR/SVI/SSVI) on
        // the ONE `mark_surface` contract path (its `smile_model` selector).
        const stagedModel = getStaged(mark.stagingId)?.model ?? "MARKET_HEDGE";
        const reply = await conn.markSurface({
          pair: ccyPairToWire(parsePairLoose(val("mk-pair"))),
          broker_quotes: [
            brokerQuoteSetToWire({
              tenorYears: tenorYearsLoose(val("mk-tenor")),
              atmVol: Number(val("mk-vol")),
              rr25: 0,
              bf25: 0,
              rr10: 0,
              bf10: 0,
              hasTenDelta: false,
            }),
          ],
          conventions: conventionsToWire(DEFAULT_CONVENTIONS),
          smile_model: smileModel.toWire(stagedModel),
        });
        const surfaceVersion = typeof reply["surface_version"] === "number"
          ? BigInt(Math.trunc(reply["surface_version"] as number))
          : 0n;
        markCommitted(mark.stagingId, surfaceVersion, "committed");
        mark = markCommittedState(mark, surfaceVersion, mark.stagingId);
        (el("mk-commit") as HTMLButtonElement).disabled = true;
        setState("mk-state", mark.status, "ok");
        el("mk-pending").textContent = mark.status;
      } catch (err) {
        markRejected(mark.stagingId, message(err));
        mark = markRejectedState(mark, message(err));
        setState("mk-state", mark.status, "bad");
        el("mk-pending").textContent = mark.status;
      }
    })();
  });

  renderRfq();
}

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function formatNanos(nanos: bigint): string {
  return nanos === 0n ? "—" : new Date(Number(nanos / 1_000_000n)).toISOString();
}

// Loose parsers for the commit body (the strict ones throw on stage already).
function parsePairLoose(raw: string): { base: string; quote: string } {
  const s = raw.trim().toUpperCase().replace("/", "");
  return { base: s.slice(0, 3), quote: s.slice(3, 6) };
}
function tenorYearsLoose(raw: string): number {
  const s = raw.trim().toUpperCase();
  const m = /^(\d+)([WMY])$/.exec(s);
  if (!m) return 1;
  const n = Number(m[1]);
  return m[2] === "W" ? (n * 7) / 365 : m[2] === "M" ? n / 12 : n;
}

// Office.onReady fires when the host is initialized; under a plain browser preview
// (no Office host) fall back to DOMContentLoaded so the pane is still interactive.
const officeGlobal = (globalThis as unknown as {
  Office?: { onReady?: (cb: () => void) => void };
}).Office;
if (officeGlobal?.onReady) {
  officeGlobal.onReady(() => boot());
} else if (typeof document !== "undefined") {
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }
}
