/**
 * ConventionChip + ConventionRow — the conventions on the FACE (GUI-DESIGN
 * principle 4). Every quote shows resolved premium/delta/cut/ATM conventions as
 * small pills; transparency is a feature, not a tooltip. Labels localize, the
 * canonical values never do (§7).
 */

import type { Conventions } from "../data/contract";
import {
  atmConvChip,
  cutChip,
  deltaConvChip,
  premiumChip,
} from "../lib/format";
import styles from "./ConventionChip.module.css";

export function ConventionChip({
  children,
  tone = "neutral",
}: {
  children: React.ReactNode;
  tone?: "neutral" | "accent";
}): React.ReactElement {
  return (
    <span className={`${styles.chip} ${tone === "accent" ? styles.accent : ""}`}>
      {children}
    </span>
  );
}

export function ConventionRow({
  conventions,
}: {
  conventions: Conventions;
}): React.ReactElement {
  return (
    <div className={styles.row} aria-label="resolved conventions">
      <ConventionChip>{deltaConvChip(conventions.deltaConvention)}</ConventionChip>
      <ConventionChip>{atmConvChip(conventions.atmConvention)}</ConventionChip>
      <ConventionChip>{cutChip(conventions.cut)}</ConventionChip>
      <ConventionChip>{premiumChip(conventions.premiumStyle)}</ConventionChip>
      <ConventionChip>
        {conventions.settlement === "DELIVERABLE" ? "deliv" : "NDO"}
      </ConventionChip>
    </div>
  );
}
