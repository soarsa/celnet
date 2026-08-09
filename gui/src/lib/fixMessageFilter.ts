/**
 * fixMessageFilter — the pure investigation logic behind the FIX Session
 * Monitor's search/filter bar. All matching is derived over the retained frame
 * buffer (never mutated): a free-text query plus structured filters (direction,
 * MsgType multi-select, a `tag=value` probe, and a "last N ms" time window),
 * ALL AND-ed together.
 *
 * The `tag=value` probe is the investigation power feature: it parses the raw
 * pipe-delimited FIX body into a tag→values map (repeating groups keep every
 * occurrence) and matches a specific FIX tag's value, so `55=912797UU9` isolates
 * exactly the frames carrying that Symbol — without the false hits a bare
 * substring search over the whole line would produce.
 *
 * Kept framework-free and side-effect-free so it is unit-testable in isolation
 * and reusable by the monitor component.
 */

import type { FixMessage, FixMsgDirection } from "../data/contract";

/** Nanoseconds per millisecond — for collapsing the capture stamp to wall-clock ms. */
const NANOS_PER_MS = 1_000_000n;

/** Direction gate: either travel direction, or only inbound / only outbound. */
export type FixDirFilter = "all" | "inbound" | "outbound";

/**
 * The composed monitor filter. Every populated field narrows the visible set;
 * an empty field is inert. The default direction is `inbound` (the monitor's
 * long-standing inbound-first posture).
 */
export interface FixMsgFilter {
  /** Free-text query, matched case-insensitively over raw + decoded label + MsgType. */
  readonly text: string;
  /** Travel-direction gate. */
  readonly direction: FixDirFilter;
  /** MsgType(35) values to keep; empty ⇒ every type. */
  readonly msgTypes: ReadonlySet<string>;
  /** A `tag=value` probe (e.g. `55=912797UU9`); blank/invalid ⇒ inert. */
  readonly tagQuery: string;
  /** Keep only frames captured within the last N ms; `null` ⇒ all time. */
  readonly sinceMs: number | null;
}

/** The inert starting filter — inbound-first, nothing else constrained. */
export const EMPTY_FIX_FILTER: FixMsgFilter = {
  text: "",
  direction: "inbound",
  msgTypes: new Set<string>(),
  tagQuery: "",
  sinceMs: null,
};

/** A parsed `tag=value` probe: a numeric FIX tag + a (possibly empty) value needle. */
export interface FixTagQuery {
  /** The FIX tag number to inspect (e.g. 55 for Symbol). */
  readonly tag: number;
  /** Substring the tag's value must contain; empty ⇒ "tag present" only. */
  readonly value: string;
}

/**
 * Parse the raw pipe-delimited FIX body into a `tag → values` map. Repeating
 * groups (e.g. several `55=` in one message) keep every occurrence so the probe
 * can match any of them. Malformed fields are skipped.
 */
export function parseFixTags(raw: string): Map<number, string[]> {
  const out = new Map<number, string[]>();
  for (const field of raw.split("|")) {
    const eq = field.indexOf("=");
    if (eq <= 0) continue;
    const tag = Number(field.slice(0, eq));
    if (!Number.isInteger(tag) || tag <= 0) continue;
    const value = field.slice(eq + 1);
    const arr = out.get(tag);
    if (arr) arr.push(value);
    else out.set(tag, [value]);
  }
  return out;
}

/**
 * Parse a raw `tag=value` probe string. Accepts `55=912797UU9` (tag + value
 * needle) or a bare `55` (tag-present). Returns `null` when the input is blank
 * or the tag is not a positive integer, so a half-typed probe is inert.
 */
export function parseTagQuery(input: string): FixTagQuery | null {
  const s = input.trim();
  if (s === "") return null;
  const eq = s.indexOf("=");
  const tagStr = eq >= 0 ? s.slice(0, eq) : s;
  const tag = Number(tagStr.trim());
  if (!Number.isInteger(tag) || tag <= 0) return null;
  const value = eq >= 0 ? s.slice(eq + 1).trim() : "";
  return { tag, value };
}

/** Does a parsed tag map satisfy the probe? */
function tagMatches(tags: Map<number, string[]>, q: FixTagQuery): boolean {
  const vals = tags.get(q.tag);
  if (!vals) return false;
  if (q.value === "") return true;
  const needle = q.value.toLowerCase();
  return vals.some((v) => v.toLowerCase().includes(needle));
}

/** Map the direction gate onto a captured frame's travel direction. */
function directionAllows(filter: FixDirFilter, dir: FixMsgDirection): boolean {
  if (filter === "inbound") return dir === "INBOUND";
  if (filter === "outbound") return dir === "OUTBOUND";
  return true;
}

/**
 * Does one captured frame satisfy the whole (AND-ed) filter, relative to `nowMs`
 * (the reference "now" for the time window — passed in so the predicate is pure)?
 */
export function messageMatches(
  m: FixMessage,
  filter: FixMsgFilter,
  nowMs: number,
): boolean {
  if (!directionAllows(filter.direction, m.direction)) return false;

  if (filter.msgTypes.size > 0 && !filter.msgTypes.has(m.msgType)) return false;

  if (filter.sinceMs !== null) {
    const ms = Number(m.epochNanos / NANOS_PER_MS);
    if (!(ms >= nowMs - filter.sinceMs)) return false;
  }

  const text = filter.text.trim().toLowerCase();
  if (text !== "") {
    // CompIDs (49/56) live in `raw`, so the free-text query covers them too.
    const hay = `${m.raw}\n${m.summary}\n${m.msgType}`.toLowerCase();
    if (!hay.includes(text)) return false;
  }

  const probe = parseTagQuery(filter.tagQuery);
  if (probe && !tagMatches(parseFixTags(m.raw), probe)) return false;

  return true;
}

/**
 * Filter a retained frame buffer down to the matching frames (a NEW array — the
 * buffer is never mutated). `nowMs` defaults to `Date.now()`.
 */
export function filterFixMessages(
  messages: readonly FixMessage[],
  filter: FixMsgFilter,
  nowMs: number = Date.now(),
): FixMessage[] {
  return messages.filter((m) => messageMatches(m, filter, nowMs));
}

/** Is any facet of the filter constraining the view (vs. the inert default)? */
export function isFixFilterActive(filter: FixMsgFilter): boolean {
  return (
    filter.text.trim() !== "" ||
    filter.direction !== "inbound" ||
    filter.msgTypes.size > 0 ||
    filter.tagQuery.trim() !== "" ||
    filter.sinceMs !== null
  );
}

/** A MsgType option for the multi-select, built from the frames actually seen. */
export interface MsgTypeOption {
  /** The FIX MsgType(35) value, e.g. `R`. */
  readonly msgType: string;
  /** Its human label, e.g. `QuoteRequest` (the server-decoded summary). */
  readonly label: string;
}

/**
 * Build the MsgType multi-select option list from the frames present in the
 * buffer — one entry per distinct MsgType, labelled with its decoded name,
 * sorted by label so the picker is stable as new types appear.
 */
export function msgTypeOptions(messages: readonly FixMessage[]): MsgTypeOption[] {
  const seen = new Map<string, string>();
  for (const m of messages) {
    if (!seen.has(m.msgType)) seen.set(m.msgType, m.summary || m.msgType);
  }
  return [...seen.entries()]
    .map(([msgType, label]) => ({ msgType, label }))
    .sort((a, b) => a.label.localeCompare(b.label) || a.msgType.localeCompare(b.msgType));
}

/** One run of raw text, flagged as a highlight hit or plain. */
export interface HighlightSegment {
  readonly text: string;
  readonly match: boolean;
}

/**
 * Split `text` into alternating plain / matched runs against a case-insensitive
 * `query`, so the row renderer can wrap the hits in `<mark>`. A blank query
 * yields a single plain run (nothing highlighted).
 */
export function highlightSegments(text: string, query: string): HighlightSegment[] {
  const q = query.trim();
  if (q === "") return [{ text, match: false }];
  const hay = text.toLowerCase();
  const needle = q.toLowerCase();
  const segs: HighlightSegment[] = [];
  let i = 0;
  for (;;) {
    const hit = hay.indexOf(needle, i);
    if (hit < 0) {
      if (i < text.length) segs.push({ text: text.slice(i), match: false });
      break;
    }
    if (hit > i) segs.push({ text: text.slice(i, hit), match: false });
    segs.push({ text: text.slice(hit, hit + needle.length), match: true });
    i = hit + needle.length;
  }
  return segs.length > 0 ? segs : [{ text, match: false }];
}

/**
 * Append `incoming` frames to `prev`, keeping at most `cap` (newest-last): the
 * bounded ring behind the retained investigation buffer. Returns `prev`
 * unchanged when nothing arrives (stable identity ⇒ no needless re-render).
 */
export function appendCapped<T>(
  prev: readonly T[],
  incoming: readonly T[],
  cap: number,
): T[] {
  if (incoming.length === 0) return prev as T[];
  const next = prev.concat(incoming);
  return next.length > cap ? next.slice(next.length - cap) : next;
}
