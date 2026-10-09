/**
 * fixMessageFilter — the pure investigation logic behind the FIX Session
 * Monitor's search/filter bar: free-text (incl. raw `tag=value` substrings),
 * MsgType multi-select, direction gate, `tag=value` probe, time window,
 * match-count, highlight spans, and the bounded retained-buffer append.
 */

import { describe, expect, it } from "vitest";

import type { FixMessage } from "../src/data/contract";
import {
  appendCapped,
  EMPTY_FIX_FILTER,
  filterFixMessages,
  highlightSegments,
  isFixFilterActive,
  messageMatches,
  msgTypeOptions,
  parseFixTags,
  parseTagQuery,
  type FixMsgFilter,
} from "../src/lib/fixMessageFilter";
import { at } from "./support";

/** Fixed reference "now" (ms) — the seed frames sit just before it. */
const NOW_MS = 1_754_000_000_000;
const NOW_NANOS = BigInt(NOW_MS) * 1_000_000n;

let seq = 0n;
function msg(
  direction: FixMessage["direction"],
  msgType: string,
  summary: string,
  raw: string,
  agoMs = 0,
): FixMessage {
  seq += 1n;
  return {
    seq,
    connectionId: "c1",
    direction,
    msgType,
    summary,
    epochNanos: NOW_NANOS - BigInt(agoMs) * 1_000_000n,
    raw,
  };
}

/** A representative captured transcript across several symbols + types. */
const SAMPLE: FixMessage[] = [
  msg("INBOUND", "A", "Logon", "8=FIX.4.4|35=A|49=CELNET_RATES|56=CELNET|34=1|10=000", 60_000),
  msg("INBOUND", "R", "QuoteRequest", "8=FIX.4.4|35=R|49=CELNET_RATES|56=CELNET|55=USD-OIS|54=1|38=25000000|10=000", 40_000),
  msg("OUTBOUND", "S", "Quote", "8=FIX.4.4|35=S|49=CELNET|56=CELNET_RATES|55=USD-OIS|132=3.912|133=3.918|10=000", 39_000),
  msg("INBOUND", "R", "QuoteRequest", "8=FIX.4.4|35=R|49=CELNET_RATES|56=CELNET|55=912797UU9|54=1|38=5000000|10=000", 20_000),
  msg("OUTBOUND", "S", "Quote", "8=FIX.4.4|35=S|49=CELNET|56=CELNET_RATES|55=912797UU9|132=96.214|133=96.238|10=000", 19_000),
  msg("INBOUND", "D", "NewOrderSingle", "8=FIX.4.4|35=D|49=CELNET_RATES|56=CELNET|11=O-1|55=912797UU9|54=1|38=5000000|44=96.238|10=000", 5_000),
  msg("OUTBOUND", "8", "ExecutionReport", "8=FIX.4.4|35=8|49=CELNET|56=CELNET_RATES|37=O-1|150=F|39=2|55=912797UU9|54=1|10=000", 4_000),
];

function withFilter(p: Partial<FixMsgFilter>): FixMsgFilter {
  return { ...EMPTY_FIX_FILTER, direction: "all", ...p };
}

describe("parseFixTags", () => {
  it("parses pipe-delimited tag=value into a tag→values map, keeping repeats", () => {
    const tags = parseFixTags("8=FIX.4.4|55=USD-OIS|55=EUR-OIS|54=1|bad|=x|10=000");
    expect(tags.get(55)).toEqual(["USD-OIS", "EUR-OIS"]);
    expect(tags.get(54)).toEqual(["1"]);
    expect(tags.get(8)).toEqual(["FIX.4.4"]);
    // Malformed fields (no tag / empty tag) are skipped.
    expect([...tags.keys()].some((k) => Number.isNaN(k))).toBe(false);
  });
});

describe("parseTagQuery", () => {
  it("parses `tag=value`, bare `tag`, and rejects junk", () => {
    expect(parseTagQuery("55=912797UU9")).toEqual({ tag: 55, value: "912797UU9" });
    expect(parseTagQuery(" 54 = 1 ")).toEqual({ tag: 54, value: "1" });
    expect(parseTagQuery("55")).toEqual({ tag: 55, value: "" });
    expect(parseTagQuery("")).toBeNull();
    expect(parseTagQuery("abc")).toBeNull();
    expect(parseTagQuery("=1")).toBeNull();
  });
});

describe("messageMatches — free text", () => {
  it("matches raw tag=value substrings case-insensitively", () => {
    const f = withFilter({ text: "55=usd-ois" });
    expect(SAMPLE.filter((m) => messageMatches(m, f, NOW_MS))).toHaveLength(2);
  });

  it("matches a bare value present in the raw (e.g. a CUSIP)", () => {
    const f = withFilter({ text: "912797UU9" });
    // 2 quotes + 1 order + 1 exec report carry that symbol.
    expect(filterFixMessages(SAMPLE, f, NOW_MS)).toHaveLength(4);
  });

  it("matches on the decoded label and the CompIDs", () => {
    expect(filterFixMessages(SAMPLE, withFilter({ text: "quoterequest" }), NOW_MS)).toHaveLength(2);
    expect(filterFixMessages(SAMPLE, withFilter({ text: "celnet_rates" }), NOW_MS)).toHaveLength(SAMPLE.length);
  });

  it("matches `35=D` free-text", () => {
    expect(filterFixMessages(SAMPLE, withFilter({ text: "35=D" }), NOW_MS)).toHaveLength(1);
  });
});

describe("messageMatches — structured filters", () => {
  it("MsgType multi-select keeps only chosen types", () => {
    const f = withFilter({ msgTypes: new Set(["R", "S"]) });
    const out = filterFixMessages(SAMPLE, f, NOW_MS);
    expect(out).toHaveLength(4);
    expect(out.every((m) => m.msgType === "R" || m.msgType === "S")).toBe(true);
  });

  it("direction gate filters inbound / outbound", () => {
    expect(filterFixMessages(SAMPLE, withFilter({ direction: "inbound" }), NOW_MS)).toHaveLength(4);
    expect(filterFixMessages(SAMPLE, withFilter({ direction: "outbound" }), NOW_MS)).toHaveLength(3);
  });

  it("tag=value probe isolates a specific Symbol without value false-positives", () => {
    const f = withFilter({ tagQuery: "55=912797UU9" });
    const out = filterFixMessages(SAMPLE, f, NOW_MS);
    expect(out).toHaveLength(4);
    expect(out.every((m) => m.raw.includes("55=912797UU9"))).toBe(true);
  });

  it("tag=value probe with a bare tag matches presence", () => {
    // Every non-Logon frame carries a Symbol(55).
    expect(filterFixMessages(SAMPLE, withFilter({ tagQuery: "55" }), NOW_MS)).toHaveLength(6);
  });

  it("Side(54)=1 tag probe isolates buy-side frames", () => {
    const out = filterFixMessages(SAMPLE, withFilter({ tagQuery: "54=1" }), NOW_MS);
    expect(out.every((m) => m.raw.includes("54=1"))).toBe(true);
    expect(out).toHaveLength(4);
  });

  it("time window keeps only recent frames", () => {
    // Only the order + exec report are within the last 10s.
    expect(filterFixMessages(SAMPLE, withFilter({ sinceMs: 10_000 }), NOW_MS)).toHaveLength(2);
  });

  it("AND-composes text + type + tag + direction", () => {
    const f = withFilter({
      direction: "inbound",
      msgTypes: new Set(["D"]),
      tagQuery: "55=912797UU9",
      text: "44=96.238",
    });
    const out = filterFixMessages(SAMPLE, f, NOW_MS);
    expect(out).toHaveLength(1);
    expect(at(out, 0).msgType).toBe("D");
  });

  it("an empty filter shows everything with the inbound-first default", () => {
    // EMPTY_FIX_FILTER defaults to inbound.
    expect(filterFixMessages(SAMPLE, EMPTY_FIX_FILTER, NOW_MS)).toHaveLength(4);
    expect(isFixFilterActive(EMPTY_FIX_FILTER)).toBe(false);
    expect(isFixFilterActive(withFilter({ text: "x" }))).toBe(true);
  });
});

describe("msgTypeOptions", () => {
  it("builds a deduped, label-sorted option list from the frames seen", () => {
    const opts = msgTypeOptions(SAMPLE);
    expect(opts.map((o) => o.msgType).sort()).toEqual(["8", "A", "D", "R", "S"]);
    expect(opts.find((o) => o.msgType === "R")?.label).toBe("QuoteRequest");
    // Sorted by human label.
    expect(opts.map((o) => o.label)).toEqual([...opts.map((o) => o.label)].sort((a, b) => a.localeCompare(b)));
  });
});

describe("highlightSegments", () => {
  it("splits into alternating plain/matched runs, case-insensitive", () => {
    const segs = highlightSegments("55=USD-OIS|54=1", "usd");
    expect(segs.map((s) => s.text).join("")).toBe("55=USD-OIS|54=1");
    expect(segs.filter((s) => s.match).map((s) => s.text)).toEqual(["USD"]);
  });

  it("a blank query yields a single plain run", () => {
    expect(highlightSegments("abc", "")).toEqual([{ text: "abc", match: false }]);
  });

  it("highlights every occurrence", () => {
    const segs = highlightSegments("aXaXa", "x");
    expect(segs.filter((s) => s.match)).toHaveLength(2);
  });
});

describe("appendCapped", () => {
  it("keeps at most `cap` items, newest-last (ring buffer)", () => {
    const seed = Array.from({ length: 10 }, (_, i) => i);
    const capped = appendCapped(seed, [10, 11, 12], 8);
    expect(capped).toHaveLength(8);
    expect(capped[0]).toBe(5);
    expect(capped[capped.length - 1]).toBe(12);
  });

  it("returns prev unchanged (same identity) when nothing arrives", () => {
    const seed = [1, 2, 3];
    expect(appendCapped(seed, [], 5)).toBe(seed);
  });

  it("does not trim when under the cap", () => {
    expect(appendCapped([1, 2], [3], 5)).toEqual([1, 2, 3]);
  });
});
