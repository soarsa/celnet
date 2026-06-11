/**
 * A minimal Node-side request/reply seam over the PRODUCTION GUI WebSocket codec
 * (`src/data/wsCodec.ts`) for the real-edge conformance spec.
 *
 * The conformance surface under test is the GUI's codec — `instrumentToWire` /
 * `marketToWire` / `conventionsToWire` / `greeksFromWire` and the type-tagged
 * snake_case frame envelope — which this client IMPORTS, never duplicates. The
 * only e2e-local code is the socket + correlation bookkeeping, mirroring the
 * production `WsConnection.request` envelope exactly (`{...body, type,
 * correlation_id}` out; route the reply by `correlation_id`; a `type: "error"`
 * frame rejects with the server's message). It exists because the production
 * `WsTransport` types against the browser's generic `MessageEvent<T>` (DOM lib),
 * which the node-lib e2e tsconfig cannot type-check — the same reason the Excel
 * conformance e2e supplies `nodeSocket.ts` around its production `Connection`.
 *
 * No reconnect, no backoff: a conformance pricing run must fail LOUDLY on a
 * drop, never silently re-dial and re-price.
 */
import type { Conventions, Greeks, Instrument, MarketContext } from "../src/data/contract";
import {
  conventionsToWire,
  greeksFromWire,
  instrumentToWire,
  marketToWire,
  parseFrame,
  serializeFrame,
  type WireObject,
} from "../src/data/wsCodec";

/** Per-call deadline covering the heaviest MC family on the server (Excel parity). */
const DEFAULT_REQUEST_TIMEOUT_MS = 90_000;
/** Bounded wait for the socket to reach OPEN against the already-booted edge. */
const OPEN_TIMEOUT_MS = 30_000;

/** A pending request/reply waiter, keyed by its correlation id. */
interface Waiter {
  resolve: (frame: WireObject) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

/** A server refusal (`type: "error"` frame) — distinct from a transport failure. */
export class EdgeRefusal extends Error {
  constructor(message: string) {
    super(message);
    this.name = "EdgeRefusal";
  }
}

/**
 * One WebSocket to the booted demo edge, request/reply only (the streaming RFS
 * frames a live edge may push are ignored — they carry no `correlation_id` we
 * minted). Node ≥ 22 provides the WHATWG `WebSocket` global the handler
 * properties below are part of.
 */
export class EdgeClient {
  private readonly ws: WebSocket;
  private readonly requestTimeoutMs: number;
  private readonly waiters = new Map<number, Waiter>();
  private nextCorrelation = 1;
  private closed = false;

  private constructor(ws: WebSocket, requestTimeoutMs: number) {
    this.ws = ws;
    this.requestTimeoutMs = requestTimeoutMs;
    ws.onmessage = (ev: MessageEvent) => {
      if (typeof ev.data !== "string") return; // the mirror is text JSON only
      this.dispatch(ev.data);
    };
    ws.onclose = () => {
      this.failAll(new Error("edge connection closed mid-conformance-run"));
    };
  }

  /** Dial the edge and resolve once the socket is OPEN (bounded). */
  static connect(
    url: string,
    requestTimeoutMs: number = DEFAULT_REQUEST_TIMEOUT_MS,
  ): Promise<EdgeClient> {
    return new Promise<EdgeClient>((resolve, reject) => {
      const ws = new WebSocket(url);
      const timer = setTimeout(() => {
        ws.close();
        reject(new Error(`e2e: socket to ${url} never opened (${OPEN_TIMEOUT_MS}ms)`));
      }, OPEN_TIMEOUT_MS);
      ws.onopen = () => {
        clearTimeout(timer);
        resolve(new EdgeClient(ws, requestTimeoutMs));
      };
      ws.onerror = () => {
        // `onclose` follows; surface a clear connect failure once, not twice.
        clearTimeout(timer);
        reject(new Error(`e2e: socket to ${url} failed to connect`));
      };
    });
  }

  private dispatch(raw: string): void {
    let frame: WireObject;
    try {
      const parsed: unknown = parseFrame(raw);
      if (!parsed || typeof parsed !== "object") return;
      frame = parsed as WireObject;
    } catch {
      return;
    }
    const corr = frame["correlation_id"];
    if (typeof corr !== "number" && typeof corr !== "bigint") return; // server push — not ours
    const waiter = this.waiters.get(Number(corr));
    if (!waiter) return;
    this.waiters.delete(Number(corr));
    clearTimeout(waiter.timer);
    if (frame["type"] === "error") {
      waiter.reject(new EdgeRefusal(String(frame["message"] ?? "server error")));
    } else {
      waiter.resolve(frame);
    }
  }

  /**
   * Issue one request/reply call with the production envelope. Rejects with
   * {@link EdgeRefusal} on a server `error` frame, or a plain `Error` on a
   * timeout / drop — a conformance run never hangs and never retries.
   */
  request(type: string, body: WireObject): Promise<WireObject> {
    if (this.closed) return Promise.reject(new Error("edge client closed"));
    const correlationId = this.nextCorrelation++;
    return new Promise<WireObject>((resolve, reject) => {
      const timer = setTimeout(() => {
        if (this.waiters.delete(correlationId)) {
          reject(new Error(`request \`${type}\` timed out (${this.requestTimeoutMs}ms)`));
        }
      }, this.requestTimeoutMs);
      this.waiters.set(correlationId, { resolve, reject, timer });
      this.ws.send(serializeFrame({ ...body, type, correlation_id: correlationId }));
    });
  }

  private failAll(err: Error): void {
    for (const [, w] of this.waiters) {
      clearTimeout(w.timer);
      w.reject(err);
    }
    this.waiters.clear();
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.failAll(new Error("edge client closed"));
    this.ws.onmessage = null;
    this.ws.onclose = null;
    try {
      this.ws.close();
    } catch {
      // already closing
    }
  }
}

/** A server-priced result: the mid valuation, its MC stderr (if any), full Greeks. */
export interface EdgePriced {
  price: number;
  /** Present (positive) ONLY for an MC-priced product — `PriceResponse.price_std_error`. */
  priceStdError?: number;
  greeks: Greeks;
}

/** Read a required child object off a reply frame. */
function child(o: WireObject, key: string): WireObject {
  const v = o[key];
  if (!v || typeof v !== "object") throw new Error(`reply missing object field \`${key}\``);
  return v as WireObject;
}

/** Read an optional finite number off a reply frame (`null`/absent ⇒ undefined). */
function optNum(o: WireObject, key: string): number | undefined {
  const v = o[key];
  if (v === null || v === undefined) return undefined;
  if (typeof v !== "number" || !Number.isFinite(v)) {
    throw new Error(`reply field \`${key}\` is not a finite number`);
  }
  return v;
}

/**
 * Price one instrument against an EXPLICIT market context over the live edge —
 * the same `price` op / `price_response` reply, encoded with the same production
 * codec calls, as `WsTransport.price`; additionally surfaces the reply's
 * `price_std_error` (the honest MC precision band the conformance gate needs).
 */
export async function priceOnEdge(
  client: EdgeClient,
  instrument: Instrument,
  market: MarketContext,
  conventions: Conventions,
): Promise<EdgePriced> {
  const reply = await client.request("price", {
    instrument: instrumentToWire(instrument),
    market: marketToWire(market),
    conventions: conventionsToWire(conventions),
  });
  const greeks = greeksFromWire(child(reply, "greeks"));
  const stdErr = optNum(reply, "price_std_error");
  return {
    price: greeks.price,
    ...(stdErr !== undefined ? { priceStdError: stdErr } : {}),
    greeks,
  };
}
