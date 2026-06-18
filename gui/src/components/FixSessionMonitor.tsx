/**
 * FixSessionMonitor — the live monitor screen for one managed FIX acceptor
 * session. Tails the captured traffic ({@link useFixMessages}) and renders each
 * frame with its direction, decoded MsgType, and raw (pipe-delimited) body.
 *
 * The ask is inbound visibility, so the **Inbound** filter is the default; an
 * operator can flip to **All** to also see the venue's responses (the captured
 * outbound frames). Auto-scroll follows the tail until the operator scrolls up or
 * pauses, so a fast session never yanks the view out from under them.
 */

import { useLayoutEffect, useMemo, useRef, useState } from "react";

import type { FixConnection, FixMessage } from "../data/contract";
import type { CelnetTransport } from "../data/transport";
import { useFixMessages } from "../hooks/useFixMessages";
import { Button } from "./Button";
import styles from "./FixSessionMonitor.module.css";

type DirFilter = "inbound" | "all";

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
  const [filter, setFilter] = useState<DirFilter>("inbound");
  const { messages, error, clear } = useFixMessages(transport, connection.id, paused);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const atBottomRef = useRef(true);

  const shown = useMemo(
    () => (filter === "inbound" ? messages.filter((m) => m.direction === "INBOUND") : messages),
    [messages, filter],
  );

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
          <div className={styles.segmented} role="group" aria-label="Direction filter">
            <button
              type="button"
              className={filter === "inbound" ? styles.segOn : styles.seg}
              aria-pressed={filter === "inbound"}
              onClick={() => setFilter("inbound")}
            >
              Inbound
            </button>
            <button
              type="button"
              className={filter === "all" ? styles.segOn : styles.seg}
              aria-pressed={filter === "all"}
              onClick={() => setFilter("all")}
            >
              All
            </button>
          </div>
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

      {error && <p className={styles.banner}>{error}</p>}

      <div className={styles.tape} ref={scrollRef} onScroll={onScroll} tabIndex={0}>
        {shown.length === 0 ? (
          <p className={styles.empty}>
            {idle
              ? "This connection is stopped — enable it to receive and monitor traffic."
              : "Waiting for session traffic…"}
          </p>
        ) : (
          <ol className={styles.list}>
            {shown.map((m) => (
              <MessageRow key={m.seq.toString()} message={m} />
            ))}
          </ol>
        )}
      </div>
      <footer className={styles.foot}>
        <span>
          {shown.length} frame{shown.length === 1 ? "" : "s"}
          {filter === "inbound" ? " (inbound)" : ""}
        </span>
        <span className={paused ? styles.pausedTag : styles.liveTag}>
          {paused ? "paused" : "live"}
        </span>
      </footer>
    </section>
  );
}

function MessageRow({ message }: { message: FixMessage }): React.ReactElement {
  const inbound = message.direction === "INBOUND";
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
      <code className={styles.raw}>{message.raw}</code>
    </li>
  );
}
