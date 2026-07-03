// Linear-rates BOOK ledger slice — the `list_rates_positions` add-in path
// (CELNET.RATESBOOK): build the request EXACTLY as the server decodes it
// (`crates/celnet-server/src/ws/codec.rs` `list_rates_positions_request_from_json`
// + `rates_risk_scope_from_json`, oracle `list_rates_positions_round_trip`),
// decode the server-owned position ledger (`rates_position_to_json`, incl. the OIS
// `side` 0/1 ⇔ PAY/RECEIVE fixed), decode the entity/book registry
// (`entity_desc_to_json` / `book_desc_to_json`), and lay the ledger out resolving
// each numeric (entity, book) key to its registry NAME (mirroring the GUI
// RatesBookWorkspace; an unknown key ⇒ `#<key>`). The add-in holds no book state —
// it is a thin client of the live `RiskService` over the one unversioned contract.
import { describe, expect, it } from "vitest";
import { Connection } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import {
  bookDescFromWire,
  entityDescFromWire,
  listBooksResponseFromWire,
  listEntitiesResponseFromWire,
  listRatesPositionsRequest,
  listRatesPositionsResponseFromWire,
  ratesPositionFromWire,
  type RatesPosition,
} from "../src/contract/riskCodec";
import { formatRatesBookSpill } from "../src/functions/shaping";

// --- in-memory socket (mirrors test/ratesRisk.test.ts) ----------------------
class FakeSocket implements WebSocketLike {
  readyState = 0;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((data: string) => void) | null = null;
  readonly sent: Record<string, unknown>[] = [];

  send(data: string): void {
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }
  close(): void {
    this.readyState = 3;
    this.onclose?.();
  }
  open(): void {
    this.readyState = 1;
    this.onopen?.();
  }
  deliver(frame: Record<string, unknown>): void {
    this.onmessage?.(JSON.stringify(frame));
  }
  sentOfType(type: string): Record<string, unknown>[] {
    return this.sent.filter((f) => f["type"] === type);
  }
}

function makeConn(sessionToken?: string): { conn: Connection; sock: FakeSocket } {
  const sock = new FakeSocket();
  let seq = 1;
  const conn = new Connection({
    url: "ws://test",
    factory: () => sock,
    stalenessWindowMs: 1000,
    clock: () => 0,
    setTimer: () => seq++,
    clearTimer: () => {},
    requestTimeoutMs: 5000,
    ...(sessionToken !== undefined ? { sessionToken } : {}),
  });
  return { conn, sock };
}

// The server-owned ledger the OIS side (0 = payer, 1 = receiver) round-trips through.
const OIS_POSITION_WIRE = {
  position_id: 7,
  entity: 1,
  book: 100,
  instrument: { ois: { tenor_years: 5, fixed_rate: 0.0405, notional: 1e8, side: 0 } },
};

describe("rates-book wire codec — the fields the engine encodes/decodes", () => {
  it("builds a list request with an explicit grant-all principal and no scope by default", () => {
    const wire = listRatesPositionsRequest({});
    // Matches the server oracle (`list_rates_positions_round_trip`): grant-all clears
    // the deny-by-default edge; no token is needed (parity with the risk RPCs).
    expect(wire["principal"]).toEqual({ grant_all: true, grants: [], denies: [] });
    expect("scope" in wire).toBe(false);
    expect("session_token" in wire).toBe(false);
  });

  it("emits the (entity, book, ccy) scope only for the present fields", () => {
    const wire = listRatesPositionsRequest({ scope: { book: 100 } });
    expect(wire["scope"]).toEqual({ book: 100 });
    const both = listRatesPositionsRequest({ scope: { entity: 1, book: 100, ccy: "USD" } });
    expect(both["scope"]).toEqual({ entity: 1, book: 100, ccy: "USD" });
  });

  it("passes an explicit entitlement principal through verbatim (deny-wins)", () => {
    const wire = listRatesPositionsRequest({
      principal: { grantAll: false, grants: [{ scopes: [{ dimension: "BOOK", value: 100n }] }], denies: [] },
    });
    expect(wire["principal"]).toEqual({
      grant_all: false,
      grants: [{ scopes: [{ dimension: 2, value: 100 }] }],
      denies: [],
    });
  });

  it("decodes a position, mapping the OIS side code back to PAY/RECEIVE fixed", () => {
    const payer = ratesPositionFromWire(OIS_POSITION_WIRE);
    expect(payer).toEqual({
      positionId: 7n,
      entity: 1,
      book: 100,
      instrument: { tenorYears: 5, fixedRate: 0.0405, notional: 1e8, direction: "PAY_FIXED" },
    });
    const receiver = ratesPositionFromWire({
      ...OIS_POSITION_WIRE,
      instrument: { ois: { tenor_years: 10, fixed_rate: 0.0418, notional: 5e7, side: 1 } },
    });
    expect(receiver.instrument.direction).toBe("RECEIVE_FIXED");
  });

  it("decodes the positions array and an empty ledger", () => {
    expect(listRatesPositionsResponseFromWire({ positions: [OIS_POSITION_WIRE] }).positions).toHaveLength(1);
    expect(listRatesPositionsResponseFromWire({}).positions).toEqual([]);
  });

  it("decodes the entity/book registry (mapping entity_key → entityKey)", () => {
    expect(entityDescFromWire({ key: 1, name: "Celnet Global Markets", code: "CGM" })).toEqual({
      key: 1,
      name: "Celnet Global Markets",
      code: "CGM",
    });
    expect(bookDescFromWire({ key: 100, name: "Rates Trading", entity_key: 1 })).toEqual({
      key: 100,
      name: "Rates Trading",
      entityKey: 1,
    });
    expect(listEntitiesResponseFromWire({ entities: [{ key: 1, name: "E", code: "E" }] })).toHaveLength(1);
    expect(listBooksResponseFromWire({ books: [{ key: 100, name: "B", entity_key: 1 }] })).toHaveLength(1);
  });
});

describe("rates-book spill layout — registry name resolution", () => {
  const POSITIONS: readonly RatesPosition[] = [
    { positionId: 7n, entity: 1, book: 100, instrument: { tenorYears: 5, fixedRate: 0.0405, notional: 1e8, direction: "PAY_FIXED" } },
    { positionId: 8n, entity: 1, book: 999, instrument: { tenorYears: 10, fixedRate: 0.0418, notional: 5e7, direction: "RECEIVE_FIXED" } },
  ];
  const entityName = (k: number): string => (k === 1 ? "CGM" : `#${k}`);
  const bookName = (k: number): string => (k === 100 ? "Rates Trading" : `#${k}`);

  it("lays out a header + one row per position with names resolved, falling back to #<key>", () => {
    const spill = formatRatesBookSpill(POSITIONS, entityName, bookName);
    expect(spill[0]).toEqual([
      "position_id",
      "entity",
      "book",
      "instrument",
      "fixed_rate",
      "notional",
      "direction",
    ]);
    expect(spill[1]).toEqual(["7", "CGM", "Rates Trading", "5y OIS", 0.0405, 1e8, "PAY_FIXED"]);
    // The unknown book key 999 falls back to `#999` (a raw number is never shown).
    expect(spill[2]).toEqual(["8", "CGM", "#999", "10y OIS", 0.0418, 5e7, "RECEIVE_FIXED"]);
    expect(String(spill[3]![0])).toMatch(/2 positions/);
    // Office.js custom functions require a rectangular 2-D return.
    const width = spill[0]!.length;
    for (const row of spill) expect(row.length).toBe(width);
  });

  it("spills an honest empty-state when the book is empty", () => {
    const spill = formatRatesBookSpill([], entityName, bookName);
    expect(spill).toHaveLength(2);
    expect(String(spill[1]![0])).toMatch(/no rates positions/);
  });
});

describe("Connection round-trip — list_rates_positions + the registry reads", () => {
  it("sends a `list_rates_positions` frame and decodes a reply matched by its stamped type (no echoed correlation)", async () => {
    const { conn, sock } = makeConn();
    sock.open();
    const p = conn.listRatesPositions(listRatesPositionsRequest({ scope: { book: 100 } }));
    const sent = sock.sentOfType("list_rates_positions")[0]!;
    expect(sent["principal"]).toEqual({ grant_all: true, grants: [], denies: [] });
    expect(sent["scope"]).toEqual({ book: 100 });
    // The server's `list_rates_positions_response` carries NO echoed correlation id
    // (`tagged` stamps only `type`), so the reply is matched by the `expect` type.
    sock.deliver({ type: "list_rates_positions_response", positions: [OIS_POSITION_WIRE] });
    const reply = await p;
    const { positions } = listRatesPositionsResponseFromWire(reply);
    expect(positions[0]!.positionId).toBe(7n);
    expect(positions[0]!.instrument.direction).toBe("PAY_FIXED");
  });

  it("rides the held session token on the registry reads and matches the `entities`/`books` types", async () => {
    const { conn, sock } = makeConn("tok-123");
    sock.open();
    const pe = conn.listEntities();
    const entSent = sock.sentOfType("list_entities")[0]!;
    expect(entSent["session_token"]).toBe("tok-123");
    sock.deliver({ type: "entities", correlation_id: entSent["correlation_id"], entities: [{ key: 1, name: "CGM", code: "CGM" }] });
    expect(listEntitiesResponseFromWire(await pe)).toEqual([{ key: 1, name: "CGM", code: "CGM" }]);

    const pb = conn.listBooks();
    const bookSent = sock.sentOfType("list_books")[0]!;
    expect(bookSent["session_token"]).toBe("tok-123");
    sock.deliver({ type: "books", correlation_id: bookSent["correlation_id"], books: [{ key: 100, name: "Rates", entity_key: 1 }] });
    expect(listBooksResponseFromWire(await pb)).toEqual([{ key: 100, name: "Rates", entityKey: 1 }]);
  });

  it("omits the session token on the registry reads when anonymous (honest server refusal)", () => {
    const { conn, sock } = makeConn();
    sock.open();
    void conn.listEntities();
    expect("session_token" in sock.sentOfType("list_entities")[0]!).toBe(false);
  });
});
