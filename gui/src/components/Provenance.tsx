/**
 * Provenance — the on-face "where did this mark come from" line (GW0).
 *
 * Encodes the platform's data-honesty for attribution: every priced/marked value
 * a trader sees can name the SOURCE that produced it (e.g. the calibrated desk
 * surface, a vendor feed, a model) and, where relevant, the calibration MODEL and
 * an as-of stamp. Extracts the pattern the SurfaceWorkspace already renders inline
 * (the calibration-family note) into one reusable primitive the InspectorStrip and
 * every workspace dock.
 *
 * Each segment is optional and rendered only when present — an absent segment is
 * simply omitted (no fabricated "—" here: this is metadata, not a data cell). If
 * NO segment is present the component renders nothing rather than an empty chrome.
 */

import styles from "./Provenance.module.css";

export function Provenance({
  /** The mark's source — e.g. "desk surface", "vendor", "mock replay". */
  source,
  /** The calibration family that produced it — e.g. "extended-surface". */
  model,
  /** A human as-of stamp — e.g. a surface version "sv 42" or an ISO time. */
  asOf,
}: {
  source?: string;
  model?: string;
  asOf?: string;
}): React.ReactElement | null {
  const segments: { key: string; label: string; value: string }[] = [];
  if (source !== undefined && source !== "") segments.push({ key: "source", label: "source", value: source });
  if (model !== undefined && model !== "") segments.push({ key: "model", label: "model", value: model });
  if (asOf !== undefined && asOf !== "") segments.push({ key: "asof", label: "as of", value: asOf });

  if (segments.length === 0) return null;

  const title = segments.map((s) => `${s.label}: ${s.value}`).join(" · ");

  return (
    <span className={styles.provenance} aria-label={`provenance — ${title}`} title={title}>
      {segments.map((s, i) => (
        <span key={s.key} className={styles.segment}>
          {i > 0 && <span className={styles.sep} aria-hidden="true">·</span>}
          <span className={styles.label}>{s.label}</span>
          <span className={styles.value}>{s.value}</span>
        </span>
      ))}
    </span>
  );
}
