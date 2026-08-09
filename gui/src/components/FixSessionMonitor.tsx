/**
 * FixSessionMonitor — the live monitor + investigation screen for one managed
 * FIX acceptor session. Tails the captured traffic ({@link useFixMessages}) and
 * renders each frame with its direction, decoded MsgType, timestamp, and raw
 * (pipe-delimited) body.
 *
 * On top of the live tail it carries a full SEARCH / FILTER bar so an admin can
 * find specific frames in the flow — a debounced free-text query (matching raw
 * `tag=value` substrings, the decoded label, and CompIDs), plus structured
 * filters (direction, a MsgType multi-select built from the frames present, a
 * `tag=value` probe, and a "last N" time window), all AND-ed. Matching is
 * derived over the retained buffer ({@link filterFixMessages}) — the buffer is
 * never mutated — so search works identically while the stream is Paused for
 * careful inspection.
 *
 * Auto-scroll follows the tail until the operator scrolls up or pauses, so a fast
 * session never yanks the view out from under them.
 */

import { useLayoutEffect, useMemo, useRef, useState } from "react";

import type { FixConnection, FixMessage } from "../data/contract";
import type { CelnetTransport } from "../data/transport";
import { useDebouncedValue } from "../hooks/useDebouncedValue";
import { useFixMessages } from "../hooks/useFixMessages";
import {
  EMPTY_FIX_FILTER,
  filterFixMessages,
  highlightSegments,
  isFixFilterActive,
  msgTypeOptions,
  type FixDirFilter,
  type FixMsgFilter,
} from "../lib/fixMessageFilter";
import { Button } from "./Button";
import styles from "./FixSessionMonitor.module.css";

/** How long typing must settle before the free-text query re-filters the buffer. */
const SEARCH_DEBOUNCE_MS = 180;

/** The time-window options for the "captured within" filter. */
const TIME_WINDOWS: ReadonlyArray<{ label: string; ms: number | null }> = [
  { label: "All time", ms: null },
  { label: "Last 15s", ms: 15_000 },
  { label: "Last 60s", ms: 60_000 },
  { label: "Last 5m", ms: 300_000 },
  { label: "Last 15m", ms: 900_000 },
];

const DIRECTIONS: ReadonlyArray<{ value: FixDirFilter; label: string }> = [
  { value: "inbound", label: "Inbound" },
  { value: "outbound", label: "Outbound" },
  { value: "all", label: "All" },
];

/** Format an epoch-nanos bigint as a wall-clock `HH:MM:SS.mmm`. */
function formatTime(epochNanos: bigint): string {
  const ms = Number(epochNanos / 1_000_000n);
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  const d = new Date(ms);
  const pad = (n: number, w = 2): string => n.toString().padStart(w, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}

export interface FixSessionMonitorProps {
  transport: CelnetTransport;
  connection: FixConnection;
  onClose: () => void;
}

export function FixSessionMonitor({
  transport,
  connection,
  onClose,
}: FixSessionMonitorProps): React.ReactElement {
  const [paused, setPaused] = useState(false);
  const [filter, setFilter] = useState<FixMsgFilter>(EMPTY_FIX_FILTER);
  const [copied, setCopied] = useState(false);
  const { messages, error, clear } = useFixMessages(transport, connection.id, paused);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const atBottomRef = useRef(true);

  // The free-text query is debounced so filtering a multi-thousand-frame buffer
  // does not run on every keystroke; the structured filters apply immediately.
  const debouncedText = useDebouncedValue(filter.text, SEARCH_DEBOUNCE_MS);
  const effective = useMemo<FixMsgFilter>(
    () => ({ ...filter, text: debouncedText }),
    [filter, debouncedText],
  );

  const typeOptions = useMemo(() => msgTypeOptions(messages), [messages]);

  const shown = useMemo(
    () => filterFixMessages(messages, effective, Date.now()),
    [messages, effective],
  );

  const active = isFixFilterActive(filter);

  // Track whether the operator is pinned to the bottom (so we only auto-follow then).
  const onScroll = (): void => {
    const el = scrollRef.current;
    if (!el) return;
    atBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  };

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el && atBottomRef.current) el.scrollTop = el.scrollHeight;
  }, [shown]);

  const patch = (p: Partial<FixMsgFilter>): void => setFilter((f) => ({ ...f, ...p }));

  const toggleType = (msgType: string): void =>
    setFilter((f) => {
      const next = new Set(f.msgTypes);
      if (next.has(msgType)) next.delete(msgType);
      else next.add(msgType);
      return { ...f, msgTypes: next };
    });

  const resetFilters = (): void => setFilter(EMPTY_FIX_FILTER);

  const copyMatches = async (): Promise<void> => {
    const text = shown.map((m) => m.raw).join("\n");
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch {
      // Clipboard denied (no user gesture / permissions) — leave the label as-is.
    }
  };

  // A live monitor of a not-running session is empty by definition — surface why.
  const idle = !connection.running;

  return (
    <section className={styles.root} aria-label={`FIX monitor for ${connection.name}`}>
      <header className={styles.head}>
        <div className={styles.heading}>
          <span className={styles.title}>Session monitor</span>
          <span className={styles.subject}>
            {connection.name}
            <span className={styles.addr}>
              {connection.running && connection.boundAddr ? connection.boundAddr : connection.bindAddr}
            </span>
          </span>
        </div>
        <div className={styles.controls}>
          <Button variant="secondary" onClick={() => setPaused((p) => !p)}>
            {paused ? "Resume" : "Pause"}
          </Button>
          <Button variant="ghost" onClick={clear}>
            Clear
          </Button>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
        </div>
      </header>

      <div className={styles.filters}>
        <div className={styles.searchRow}>
          <span className={styles.field}>
            <span className={styles.glyph} aria-hidden="true">
              ⌕
            </span>
            <input
              type="search"
              className={styles.search}
              value={filter.text}
              aria-label="Search FIX messages"
              placeholder="Search raw FIX, a decoded type, or tag=value…"
              spellCheck={false}
              autoComplete="off"
              onChange={(e) => patch({ text: e.target.value })}
            />
            {filter.text !== "" && (
              <button
                type="button"
                className={styles.clear}
                aria-label="Clear search"
                onClick={() => patch({ text: "" })}
              >
                ×
              </button>
            )}
          </span>
          <span className={styles.count} aria-live="polite">
            showing {shown.length} of {messages.length}
          </span>
          <Button
            variant="ghost"
            onClick={() => void copyMatches()}
            disabled={shown.length === 0}
            title="Copy every matching raw FIX message to the clipboard"
          >
            {copied ? "Copied ✓" : `Copy ${shown.length}`}
          </Button>
        </div>

        <div className={styles.controlRow}>
          <div className={styles.segmented} role="group" aria-label="Direction filter">
            {DIRECTIONS.map((d) => (
              <button
                key={d.value}
                type="button"
                className={filter.direction === d.value ? styles.segOn : styles.seg}
                aria-pressed={filter.direction === d.value}
                onClick={() => patch({ direction: d.value })}
              >
                {d.label}
              </button>
            ))}
          </div>

          <label className={styles.tagField}>
            <span className={styles.tagLabel}>tag=value</span>
            <input
              type="text"
              className={styles.tagInput}
              value={filter.tagQuery}
              aria-label="Filter by FIX tag and value, e.g. 55=912797UU9"
              placeholder="e.g. 55=912797UU9"
              spellCheck={false}
              autoComplete="off"
              onChange={(e) => patch({ tagQuery: e.target.value })}
            />
          </label>

          <label className={styles.timeField}>
            <span className={styles.tagLabel}>Within</span>
            <select
              className={styles.select}
              value={filter.sinceMs === null ? "" : String(filter.sinceMs)}
              aria-label="Time window"
              onChange={(e) =>
                patch({ sinceMs: e.target.value === "" ? null : Number(e.target.value) })
              }
            >
              {TIME_WINDOWS.map((w) => (
                <option key={w.label} value={w.ms === null ? "" : String(w.ms)}>
                  {w.label}
                </option>
              ))}
            </select>
          </label>

          {active && (
            <button type="button" className={styles.reset} onClick={resetFilters}>
              Reset filters
            </button>
          )}
        </div>

        {typeOptions.length > 0 && (
          <div className={styles.typeRow} role="group" aria-label="Message type filter">
            <span className={styles.typesLabel}>Types</span>
            {typeOptions.map((o) => {
              const on = filter.msgTypes.has(o.msgType);
              return (
                <button
                  key={o.msgType}
                  type="button"
                  className={on ? styles.chipOn : styles.chip}
                  aria-pressed={on}
                  onClick={() => toggleType(o.msgType)}
                  title={`${o.label} (${o.msgType})`}
                >
                  <span className={styles.chipCode}>{o.msgType}</span>
                  {o.label}
                </button>
              );
            })}
          </div>
        )}

        <p className={styles.hint}>
          Search raw FIX, a decoded type, or <code>tag=value</code> — e.g.{" "}
          <code>55=912797UU9</code>, <code>35=D</code>, or <code>QuoteRequest</code>.
        </p>
      </div>

      {error && <p className={styles.banner}>{error}</p>}

      <div className={styles.tape} ref={scrollRef} onScroll={onScroll} tabIndex={0}>
        {shown.length === 0 ? (
          <p className={styles.empty}>
            {messages.length > 0 && active
              ? "No frames match the current search — adjust or reset the filters."
              : idle
                ? "This connection is stopped — enable it to receive and monitor traffic."
                : "Waiting for session traffic…"}
          </p>
        ) : (
          <ol className={styles.list}>
            {shown.map((m) => (
              <MessageRow key={m.seq.toString()} message={m} query={debouncedText} />
            ))}
          </ol>
        )}
      </div>
      <footer className={styles.foot}>
        <span>
          {active ? (
            <>
              {shown.length} match{shown.length === 1 ? "" : "es"} of {messages.length} retained
            </>
          ) : (
            <>
              {messages.length} frame{messages.length === 1 ? "" : "s"} retained
            </>
          )}
        </span>
        <span className={paused ? styles.pausedTag : styles.liveTag}>
          {paused ? "paused" : "live"}
        </span>
      </footer>
    </section>
  );
}

function MessageRow({
  message,
  query,
}: {
  message: FixMessage;
  query: string;
}): React.ReactElement {
  const inbound = message.direction === "INBOUND";
  const segments = highlightSegments(message.raw, query);
  return (
    <li className={styles.row}>
      <span className={styles.time}>{formatTime(message.epochNanos)}</span>
      <span
        className={`${styles.dir} ${inbound ? styles.dirIn : styles.dirOut}`}
        title={inbound ? "inbound" : "outbound"}
      >
        {inbound ? "▼ in" : "▲ out"}
      </span>
      <span className={styles.type}>
        <span className={styles.typeCode}>{message.msgType || "?"}</span>
        <span className={styles.typeLabel}>{message.summary}</span>
      </span>
      <code className={styles.raw}>
        {segments.map((s, i) =>
          s.match ? (
            <mark key={i} className={styles.hit}>
              {s.text}
            </mark>
          ) : (
            <span key={i}>{s.text}</span>
          ),
        )}
      </code>
      <button
        type="button"
        className={styles.copyRow}
        aria-label="Copy this raw FIX message"
        title="Copy this raw FIX message"
        onClick={() => void navigator.clipboard?.writeText(message.raw).catch(() => {})}
      >
        ⧉
      </button>
    </li>
  );
}
