/**
 * Non-deliverable forward (NDF) — the third W2 linear product (the GUI end of the
 * `celnet-linear` leaf). Risk-neutral PV is identical to a deliverable forward of
 * equal terms; only the settlement mechanics differ (cash-settled in the
 * convertible currency at a named fixing). Valid ONLY for a NON-DELIVERABLE
 * underlying. The `fixing` selector names the published settlement-rate option —
 * IDENTITY ONLY; the live fixing VALUE is an estate-gated feed, never sourced
 * in-repo. Follows the {@link asianSpec} template.
 */
import type { FixingSource, Instrument, Side } from "../data/contract";
import { ndfInstrument, type NdfTerms } from "../data/seed";
import { fmtRate } from "../lib/format";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

import { NumberField } from "../components/NumberField";

/**
 * The enumerated published settlement-rate options (`FixingSource`), each with its
 * trader-facing label. Identity only — names *which* published rate the NDF fixes
 * against (EMTA / ISDA per-currency template), never a fixing value.
 */
export const FIXING_SOURCES: { value: FixingSource; label: string }[] = [
  { value: "KRW_KFTC18", label: "KRW · KFTC18" },
  { value: "TWD_TAIPEI", label: "TWD · Taipei Forex" },
  { value: "INR_RBI_REF", label: "INR · RBI reference" },
  { value: "BRL_PTAX", label: "BRL · BCB PTAX" },
  { value: "CLP_DOLAR_OBS", label: "CLP · Dólar Observado" },
  { value: "COP_TRM", label: "COP · TRM" },
];

/** The NDF ticket inputs. */
export interface NdfInputs {
  /** Contract (forward) rate; `0` ⇒ default to the ATM-forward when shown. */
  contractRate: number;
  /** The direction taken (`BUY` = long the base/asset forward; `SELL` = short). */
  side: Side;
  /** The published settlement-rate option fixed against (identity only). */
  fixing: FixingSource;
}

/** The default NDF inputs (a BUY at the ATM-forward, KRW KFTC18 fixing). */
export const DEFAULT_NDF: NdfInputs = {
  contractRate: 0,
  side: "BUY",
  fixing: "KRW_KFTC18",
};

/**
 * Build the `NdfTerms` from the ticket inputs, defaulting a `0` contract rate to
 * the ATM-forward (the at-market rate ⇒ a zero-PV trade) and settling in the
 * pair's quote currency (the convertible leg the net cash is paid in).
 */
export function ndfTerms(n: NdfInputs, atmForward: number, settlementCcy: string): NdfTerms {
  return {
    contractRate: n.contractRate > 0 ? n.contractRate : atmForward,
    side: n.side,
    fixing: n.fixing,
    settlementCcy,
  };
}

function NdfInputBlock({ value, onChange, ctx }: InputBlockProps<NdfInputs>) {
  const { atmForward, pipDecimals, pair } = ctx;
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Direction</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="direction">
          {(["BUY", "SELL"] as Side[]).map((sd) => (
            <button
              key={sd}
              role="tab"
              aria-selected={value.side === sd}
              className={`${styles.modeTab} ${value.side === sd ? styles.modeActive : ""}`}
              onClick={() => onChange({ ...value, side: sd })}
            >
              {sd === "BUY" ? "Buy" : "Sell"}
            </button>
          ))}
        </div>
        <label className={styles.productField}>
          <span>Rate</span>
          <NumberField
            className="num"
            min={0}
            step={Math.pow(10, -pipDecimals)}
            value={value.contractRate}
            aria-label="contract rate"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) =>
              onChange({ ...value, contractRate: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>{value.contractRate > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
        </label>
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Fixing</span>
        <label className={styles.productField}>
          <span>Settlement rate</span>
          <select
            value={value.fixing}
            aria-label="fixing source"
            onChange={(ev) => onChange({ ...value, fixing: ev.target.value as FixingSource })}
          >
            {FIXING_SOURCES.map((fs) => (
              <option key={fs.value} value={fs.value}>
                {fs.label}
              </option>
            ))}
          </select>
        </label>
        <span className={styles.productLabel}>settles {pair.quote}</span>
      </div>
      <p className={styles.productNote}>
        Non-deliverable forward — same PV as a deliverable forward of equal terms
        (`side · notional · df(t) · (F − K)`), cash-settled in {pair.quote} at the named
        fixing. Closed-form, exact (no Monte-Carlo standard error). Requires a NON-DELIVERABLE
        pair. Only the fixing IDENTITY is on the wire — the live fixing value is an
        estate-gated feed, never sourced here.
      </p>
    </div>
  );
}

/** The non-deliverable-forward {@link ProductSpec}. */
export const ndfSpec = defineProduct<NdfInputs>({
  id: "NDF",
  label: "NDF (non-deliverable)",
  group: "Linear (forwards & swaps)",
  assetClass: "FX",
  summary: "Non-deliverable forward — cash-settled at a named fixing; forward PV economics.",
  keywords: ["ndf", "non-deliverable", "fixing", "cash settled", "em", "linear"],
  kind: "ndf",
  defaults: DEFAULT_NDF,
  allowedModels: ["DEFAULT"],
  toInstrument: (inputs: NdfInputs, ctx): Instrument =>
    withTenorAndModel(
      ndfInstrument(
        ctx.pair,
        ctx.tenorYears,
        ctx.notionalMm,
        ndfTerms(inputs, ctx.atmForward, ctx.pair.quote),
      ),
      ctx,
    ),
  InputBlock: NdfInputBlock,
});
