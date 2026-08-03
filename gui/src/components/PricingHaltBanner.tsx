/**
 * PricingHaltBanner — the prominent, firm-wide "pricing is halted" banner. Mounted
 * at the Shell level so it is visible across every workspace, and driven purely by
 * the live {@link usePricingControl} slice (the server broadcasts the halt state to
 * ALL clients), so it shows to EVERY user — including those who cannot operate the
 * kill-switch. Renders nothing when pricing is live.
 *
 * Severity mirrors the halt scope: an outbound-only halt is a WARN (clients are not
 * being quoted, but the books keep aggregating); a full halt is a DANGER (inbound
 * aggregation is stopped too). The danger banner is an assertive `role="alert"`; the
 * warn a polite `role="status"`.
 *
 * When pricing is live it renders a zero-height, aria-hidden placeholder (NOT null) so
 * it keeps its `.main` grid row and the workspace canvas below it stays full-height.
 */

import { usePricingControl, type HaltLevel } from "../app/PricingControlProvider";
import styles from "./PricingHaltBanner.module.css";

/** The banner copy + severity for each halt level (`none` ⇒ no banner). */
function bannerFor(level: HaltLevel): { danger: boolean; title: string; detail: string } | null {
  switch (level) {
    case "all":
      return {
        danger: true,
        title: "ALL PRICING HALTED",
        detail: "inbound aggregation + outbound quoting stopped",
      };
    case "outbound":
      return {
        danger: false,
        title: "OUTBOUND PRICING HALTED",
        detail: "clients are not being quoted",
      };
    case "inbound":
      return {
        danger: false,
        title: "INBOUND AGGREGATION HALTED",
        detail: "LP pricing is not updating the books",
      };
    case "none":
      return null;
  }
}

export function PricingHaltBanner(): React.ReactElement {
  const { level } = usePricingControl();
  const banner = bannerFor(level);
  // When pricing is live we must STILL render a node so the banner keeps its `.main`
  // grid slot (row 1). Returning null here vacates the cell and the remaining four
  // children auto-flow up a row — collapsing the `1fr` workspace canvas onto the 44px
  // title row (empty-void + clipped sub-header). An empty aria-hidden placeholder sizes
  // the `auto` row to 0, so the layout is byte-identical to an unhalted shell.
  if (banner === null) return <div aria-hidden className={styles.collapsed} />;
  return (
    <div
      className={`${styles.banner} ${banner.danger ? styles.danger : styles.warn}`}
      role={banner.danger ? "alert" : "status"}
      aria-live={banner.danger ? "assertive" : "polite"}
    >
      <span className={styles.glyph} aria-hidden>
        {"⚠"}
      </span>
      <span className={styles.title}>{banner.title}</span>
      <span className={styles.detail}>— {banner.detail}</span>
    </div>
  );
}
