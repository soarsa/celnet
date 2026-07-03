// Instrument reference-data slice — the `list_instruments` / `get_instrument`
// add-in path (CELNET.INSTRUMENTS): decode the AuthService roster EXACTLY as the
// server encodes it (`crates/celnet-server/src/ws/codec.rs` `instrument_def_to_json`
// + `external_id_to_json` + `family_to_json`), detecting the single family
// sub-object in the server's `family_from_json` order and preserving its terms
// verbatim, and lay the roster out uniformly. The add-in holds no registry — it is a
// thin read-only client of the live `AuthService` over the one unversioned contract
// (create/update/delete are admin-only and are not part of this surface).
import { describe, expect, it } from "vitest";
import { Connection } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import {
  instrumentDefFromWire,
  instrumentResponseFromWire,
  instrumentsResponseFromWire,
} from "../src/contract/referenceDataCodec";
import { formatInstrumentsSpill } from "../src/functions/shaping";

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

// An OIS definition exactly as `instrument_def_to_json` + `family_to_json(Ois)` frame it.
const OIS_DEF_WIRE = {
  instrument_id: "usd-sofr-ois-10y",
  name: "USD SOFR OIS 10Y",
  description: "10Y SOFR overnight-indexed swap",
  currency: "USD",
  external_ids: [{ scheme: "ticker", value: "USOSFR10" }],
  ois: {
    tenor: "10Y",
    index: "SOFR",
    fixed_frequency: "annual",
    fixed_day_count: "act_360",
    float_day_count: "act_360",
    business_day_convention: "modified_following",
    calendars: ["united_states"],
    spot_lag_days: 2,
  },
};

// A bond definition — exercises the broken-date term rendering + a bond-only day count.
const BOND_DEF_WIRE = {
  instrument_id: "us-t-10y",
  name: "UST 10Y",
  description: "",
  currency: "USD",
  external_ids: [],
  bond: {
    issuer: "US Treasury",
    coupon_rate: 0.04,
    coupon_type: "fixed",
    coupon_frequency: "semi_annual",
    day_count: "act_act",
    maturity_date: { year: 2036, month: 5, day: 15 },
    redemption: 100,
    calendars: ["united_states"],
  },
};

describe("instrument-def wire codec — the fields the server encodes", () => {
  it("decodes the base fields + the single family sub-object (verbatim terms)", () => {
    const def = instrumentDefFromWire(OIS_DEF_WIRE);
    expect(def.instrumentId).toBe("usd-sofr-ois-10y");
    expect(def.name).toBe("USD SOFR OIS 10Y");
    expect(def.description).toBe("10Y SOFR overnight-indexed swap");
    expect(def.currency).toBe("USD");
    expect(def.externalIds).toEqual([{ scheme: "ticker", value: "USOSFR10" }]);
    expect(def.family).toBe("ois");
    // The family terms are preserved verbatim (snake_case keys, unchanged values).
    expect(def.terms).toEqual(OIS_DEF_WIRE.ois);
  });

  it("detects each family by its sub-object key, incl. a bond with broken-date terms", () => {
    expect(instrumentDefFromWire(BOND_DEF_WIRE).family).toBe("bond");
    expect(instrumentDefFromWire(BOND_DEF_WIRE).terms["maturity_date"]).toEqual({ year: 2036, month: 5, day: 15 });
    const deposit = instrumentDefFromWire({
      instrument_id: "usd-depo-3m",
      name: "USD 3M Depo",
      currency: "USD",
      deposit: { index: "SOFR", tenor: "3M", day_count: "act_360", business_day_convention: "following", calendars: [], spot_lag_days: 2 },
    });
    expect(deposit.family).toBe("deposit");
    // A frame with no family (never emitted by the server) decodes gracefully.
    const bare = instrumentDefFromWire({ instrument_id: "x", name: "x", currency: "USD" });
    expect(bare.family).toBe("");
    expect(bare.terms).toEqual({});
  });

  it("decodes the roster + the single-instrument responses (null when absent)", () => {
    expect(instrumentsResponseFromWire({ instruments: [OIS_DEF_WIRE, BOND_DEF_WIRE] })).toHaveLength(2);
    expect(instrumentsResponseFromWire({})).toEqual([]);
    expect(instrumentResponseFromWire({ instrument: OIS_DEF_WIRE })?.family).toBe("ois");
    expect(instrumentResponseFromWire({ instrument: null })).toBeNull();
    expect(instrumentResponseFromWire({})).toBeNull();
  });
});

describe("instrument reference-data spill layout", () => {
  it("lays out a header + one row per definition (family + rendered external ids + terms)", () => {
    const spill = formatInstrumentsSpill([instrumentDefFromWire(OIS_DEF_WIRE)]);
    expect(spill[0]).toEqual([
      "instrument_id",
      "name",
      "family",
      "currency",
      "description",
      "external_ids",
      "terms",
    ]);
    const row = spill[1]!;
    expect(row[0]).toBe("usd-sofr-ois-10y");
    expect(row[2]).toBe("ois");
    expect(row[5]).toBe("ticker=USOSFR10");
    // Terms render as `key=value; …`, arrays as `[a, b]`.
    expect(String(row[6])).toMatch(/tenor=10Y/);
    expect(String(row[6])).toMatch(/index=SOFR/);
    expect(String(row[6])).toMatch(/calendars=\[united_states\]/);
    expect(String(spill[2]![0])).toMatch(/1 instrument\b/);
    const width = spill[0]!.length;
    for (const r of spill) expect(r.length).toBe(width);
  });

  it("renders a broken-date term as YYYY-MM-DD and a blank description/ids as —", () => {
    const spill = formatInstrumentsSpill([instrumentDefFromWire(BOND_DEF_WIRE)]);
    const row = spill[1]!;
    expect(row[4]).toBe("—"); // blank description
    expect(row[5]).toBe("—"); // no external ids
    expect(String(row[6])).toMatch(/maturity_date=2036-05-15/);
  });

  it("spills an honest empty-state when the roster (or a get by id) is empty", () => {
    const spill = formatInstrumentsSpill([]);
    expect(spill).toHaveLength(2);
    expect(String(spill[1]![0])).toMatch(/no instruments registered/);
  });
});

describe("Connection round-trip — list_instruments + get_instrument", () => {
  it("sends `list_instruments` with the held token and decodes an `instruments` reply", async () => {
    const { conn, sock } = makeConn("tok-abc");
    sock.open();
    const p = conn.listInstruments();
    const sent = sock.sentOfType("list_instruments")[0]!;
    expect(sent["session_token"]).toBe("tok-abc");
    sock.deliver({ type: "instruments", correlation_id: sent["correlation_id"], instruments: [OIS_DEF_WIRE] });
    const defs = instrumentsResponseFromWire(await p);
    expect(defs[0]!.instrumentId).toBe("usd-sofr-ois-10y");
    expect(defs[0]!.family).toBe("ois");
  });

  it("sends `get_instrument` with the id + token and decodes an `instrument` reply", async () => {
    const { conn, sock } = makeConn("tok-abc");
    sock.open();
    const p = conn.getInstrument("us-t-10y");
    const sent = sock.sentOfType("get_instrument")[0]!;
    expect(sent["instrument_id"]).toBe("us-t-10y");
    expect(sent["session_token"]).toBe("tok-abc");
    sock.deliver({ type: "instrument", correlation_id: sent["correlation_id"], instrument: BOND_DEF_WIRE });
    expect(instrumentResponseFromWire(await p)?.family).toBe("bond");
  });

  it("omits the token when anonymous (the server honestly refuses the read)", () => {
    const { conn, sock } = makeConn();
    sock.open();
    void conn.listInstruments();
    expect("session_token" in sock.sentOfType("list_instruments")[0]!).toBe(false);
  });
});
