/**
 * InspectorStrip — the per-Panel analytics-config header shell (GW0).
 *
 * Every workspace docks the SAME strip: a declarative row of analytics-config
 * segments in one canonical order — model · measures · axes · trend · columns ·
 * density · saved-view — followed by the always-present <Provenance> region. A
 * lane populates ONLY the segments that apply to it (a vol cube has axes; a ticket
 * does not), so the strip is a composable container, not a fixed toolbar; GW1+
 * fills the segment content (model chips, measure pickers, the density toggle, …).
 *
 * The strip is the analytics toolbar for its panel, so it carries `role="toolbar"`
 * (a labelled group of controls) — the per-segment controls the lanes inject keep
 * their own roles. Provenance lives at the trailing edge as the honest "where did
 * this come from" stamp, and is rendered even when empty-by-omission so its slot is
 * a stable part of the strip's grammar.
 */

import styles from "./InspectorStrip.module.css";

/** The canonical analytics-config segment identities, in render order. */
export const INSPECTOR_SEGMENTS = [
  "model",
  "measures",
  "axes",
  "trend",
  "columns",
  "density",
  "saved-view",
] as const;

export type InspectorSegmentId = (typeof INSPECTOR_SEGMENTS)[number];

export interface InspectorSegment {
  /** Which canonical analytics axis this segment configures. */
  id: InspectorSegmentId;
  /** Optional uppercase eyebrow label shown before the control(s). */
  label?: string;
  /** The control(s) the lane injects for this segment. */
  content: React.ReactNode;
}

const SEGMENT_ORDER: Record<InspectorSegmentId, number> = INSPECTOR_SEGMENTS.reduce(
  (acc, id, i) => {
    acc[id] = i;
    return acc;
  },
  {} as Record<InspectorSegmentId, number>,
);

export function InspectorStrip({
  /** An accessible name for the strip (e.g. the panel/workspace it configures). */
  label,
  /** The analytics-config segments this lane wants; rendered in canonical order. */
  segments = [],
  /** The trailing <Provenance> (or any provenance node). Always given a slot. */
  provenance,
}: {
  label: string;
  segments?: InspectorSegment[];
  provenance?: React.ReactNode;
}): React.ReactElement {
  // Sort defensively into the canonical order so lanes may declare segments in any
  // order and the strip's grammar (model→…→saved-view) is always consistent.
  const ordered = [...segments].sort((a, b) => SEGMENT_ORDER[a.id] - SEGMENT_ORDER[b.id]);

  return (
    <div className={styles.strip} role="toolbar" aria-label={`${label} analytics`}>
      {ordered.map((seg) => (
        <div key={seg.id} className={styles.segment} data-segment={seg.id}>
          {seg.label !== undefined && seg.label !== "" && (
            <span className={styles.segLabel}>{seg.label}</span>
          )}
          <div className={styles.segBody}>{seg.content}</div>
        </div>
      ))}
      <div className={styles.provenance} data-segment="provenance">
        {provenance}
      </div>
    </div>
  );
}
