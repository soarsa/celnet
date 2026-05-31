/**
 * DatePicker — a dependency-free month-grid calendar for selecting an arbitrary
 * BROKEN-DATE expiry (TicketWorkspace, broken-date ticket lane). Emits a
 * `BrokenDate` (year/month/day). Keyboard-first and reduced-motion aware; design
 * tokens only. It picks a CALENDAR DATE — it does NOT resolve a settlement/expiry
 * year fraction or a business-day-adjusted delivery date (that day-count + spot-lag
 * calendar logic is server-owned, in celnet-calendar). The consumer derives the
 * pricing year fraction and labels the resolution honestly.
 */

import { useMemo, useState } from "react";
import type { BrokenDate } from "../data/contract";
import styles from "./DatePicker.module.css";

const MONTH_NAMES = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
] as const;
const WEEKDAYS = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] as const;

export interface DatePickerProps {
  /** The currently selected date, if any. */
  value: BrokenDate | null;
  /** Called with the newly picked date. */
  onChange: (date: BrokenDate) => void;
  /** Earliest selectable date (inclusive); dates before this are disabled. */
  min: BrokenDate;
  /** Latest selectable date (inclusive); dates after this are disabled. */
  max: BrokenDate;
}

/** A BrokenDate as a UTC `Date` at midnight (calendar-only, no clock/time-zone math). */
function toUtc(d: BrokenDate): Date {
  return new Date(Date.UTC(d.year, d.month - 1, d.day));
}
function fromUtc(dt: Date): BrokenDate {
  return { year: dt.getUTCFullYear(), month: dt.getUTCMonth() + 1, day: dt.getUTCDate() };
}
function sameDay(a: BrokenDate, b: BrokenDate): boolean {
  return a.year === b.year && a.month === b.month && a.day === b.day;
}
/** Monday-based weekday index (0=Mon … 6=Sun) for a UTC date. */
function mondayIndex(dt: Date): number {
  return (dt.getUTCDay() + 6) % 7;
}

export function DatePicker({ value, onChange, min, max }: DatePickerProps): React.ReactElement {
  // The visible month: anchor on the selection, else the min (first valid month).
  const [view, setView] = useState<{ year: number; month: number }>(() => {
    const anchor = value ?? min;
    return { year: anchor.year, month: anchor.month };
  });

  const minMs = toUtc(min).getTime();
  const maxMs = toUtc(max).getTime();

  const grid = useMemo(() => {
    const first = new Date(Date.UTC(view.year, view.month - 1, 1));
    const lead = mondayIndex(first);
    const daysInMonth = new Date(Date.UTC(view.year, view.month, 0)).getUTCDate();
    const cells: ({ date: BrokenDate; ms: number } | null)[] = [];
    for (let i = 0; i < lead; i += 1) cells.push(null);
    for (let day = 1; day <= daysInMonth; day += 1) {
      const dt = new Date(Date.UTC(view.year, view.month - 1, day));
      cells.push({ date: fromUtc(dt), ms: dt.getTime() });
    }
    while (cells.length % 7 !== 0) cells.push(null);
    return cells;
  }, [view]);

  const step = (delta: number): void => {
    const m = view.month - 1 + delta;
    const year = view.year + Math.floor(m / 12);
    const month = ((m % 12) + 12) % 12 + 1;
    setView({ year, month });
  };

  // Disable month-step buttons once the whole next/prev month is out of range.
  const monthStartMs = new Date(Date.UTC(view.year, view.month - 1, 1)).getTime();
  const monthEndMs = new Date(Date.UTC(view.year, view.month, 0)).getTime();
  const canPrev = monthStartMs > minMs;
  const canNext = monthEndMs < maxMs;

  return (
    <div className={styles.cal} role="group" aria-label="expiry date picker">
      <div className={styles.header}>
        <button
          type="button"
          className={styles.nav}
          onClick={() => step(-1)}
          disabled={!canPrev}
          aria-label="previous month"
        >
          ‹
        </button>
        <span className={`${styles.monthLabel}`}>
          {MONTH_NAMES[view.month - 1]} <span className="num">{view.year}</span>
        </span>
        <button
          type="button"
          className={styles.nav}
          onClick={() => step(1)}
          disabled={!canNext}
          aria-label="next month"
        >
          ›
        </button>
      </div>
      <div className={styles.weekdays}>
        {WEEKDAYS.map((w) => (
          <span key={w} className={styles.weekday}>
            {w}
          </span>
        ))}
      </div>
      <div className={styles.grid}>
        {grid.map((cell, i) => {
          if (!cell) return <span key={`pad-${i}`} className={styles.pad} />;
          const disabled = cell.ms < minMs || cell.ms > maxMs;
          const selected = value !== null && sameDay(cell.date, value);
          return (
            <button
              key={`${cell.date.year}-${cell.date.month}-${cell.date.day}`}
              type="button"
              className={`num ${styles.day} ${selected ? styles.daySelected : ""}`}
              disabled={disabled}
              aria-pressed={selected}
              onClick={() => onChange(cell.date)}
            >
              {cell.date.day}
            </button>
          );
        })}
      </div>
    </div>
  );
}
