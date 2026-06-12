/**
 * GreeksStrip — Δ Γ ν Θ on the face, with vanna/volga and the full 14-Greek set
 * on expand (GUI-DESIGN §3.5). Tabular, sign-aware, glyph-labelled. The labels
 * use the Greek glyphs the desk reads; provenance/method names never appear.
 */

import { useState } from "react";
import type { Greeks } from "../data/contract";
import type { AssetClass } from "../products/types";
import { fmtSigned } from "../lib/format";
import styles from "./GreeksStrip.module.css";

interface GreekDef {
  glyph: string;
  label: string;
  pick: (g: Greeks) => number;
  decimals?: number;
}

const PRIMARY: GreekDef[] = [
  { glyph: "Δ", label: "delta (spot)", pick: (g) => g.deltaSpot },
  { glyph: "Γ", label: "gamma", pick: (g) => g.gamma, decimals: 4 },
  { glyph: "ν", label: "vega", pick: (g) => g.vega, decimals: 4 },
  { glyph: "Θ", label: "theta", pick: (g) => g.theta, decimals: 5 },
];

/** The class-INDEPENDENT secondary Greeks (the cross-derivatives). */
const SECONDARY_BASE: GreekDef[] = [
  { glyph: "vanna", label: "vanna", pick: (g) => g.vanna, decimals: 4 },
  { glyph: "volga", label: "volga", pick: (g) => g.volga, decimals: 4 },
  { glyph: "charm", label: "charm", pick: (g) => g.charm, decimals: 5 },
  { glyph: "speed", label: "speed", pick: (g) => g.speed, decimals: 5 },
  { glyph: "zomma", label: "zomma", pick: (g) => g.zomma, decimals: 5 },
  { glyph: "color", label: "color", pick: (g) => g.color, decimals: 5 },
];

/**
 * The two rate-rho Greeks, relabelled to the asset class's REAL carry identity. The
 * wire fields are unchanged — `rhoDom` is the discount-rate rho, `rhoFor` is the
 * carry rho the server computes (`b = r − carry`) — but their MEANING differs by
 * class, so an equity never reads "rho foreign": FX/metal show a two-rate pair
 * (domestic/foreign); equity shows rate + dividend-yield rho; commodity rate +
 * net-carry rho; crypto rate + funding rho. Labels only — no new math, all 14
 * Greeks are always shown.
 */
function rhoGreeksFor(cls: AssetClass): GreekDef[] {
  const rate: GreekDef = { glyph: "ρ", label: "rho (rate)", pick: (g) => g.rhoDom, decimals: 4 };
  switch (cls) {
    case "EQUITY":
      return [rate, { glyph: "ρq", label: "rho (dividend yield)", pick: (g) => g.rhoFor, decimals: 4 }];
    case "COMMODITY":
      return [rate, { glyph: "ρc", label: "rho (net carry)", pick: (g) => g.rhoFor, decimals: 4 }];
    case "CRYPTO":
      return [rate, { glyph: "ρƒ", label: "rho (funding)", pick: (g) => g.rhoFor, decimals: 4 }];
    case "FX":
    case "METAL":
    default:
      return [
        { glyph: "ρd", label: "rho domestic", pick: (g) => g.rhoDom, decimals: 4 },
        { glyph: "ρf", label: "rho foreign", pick: (g) => g.rhoFor, decimals: 4 },
      ];
  }
}

function GreekCell({ def, g }: { def: GreekDef; g: Greeks }): React.ReactElement {
  return (
    <span className={styles.cell} title={def.label}>
      <span className={styles.glyph}>{def.glyph}</span>
      <span className="num">{fmtSigned(def.pick(g), def.decimals ?? 3)}</span>
    </span>
  );
}

export function GreeksStrip({
  greeks,
  assetClass = "FX",
}: {
  greeks: Greeks;
  /** The active underlier's class — relabels the rate-rho Greeks (FX default). */
  assetClass?: AssetClass;
}): React.ReactElement {
  const [expanded, setExpanded] = useState(false);
  const secondary = [...SECONDARY_BASE, ...rhoGreeksFor(assetClass)];
  return (
    <div className={styles.strip}>
      <div className={styles.primary}>
        {PRIMARY.map((def) => (
          <GreekCell key={def.label} def={def} g={greeks} />
        ))}
        <button
          className={styles.toggle}
          onClick={() => setExpanded((e) => !e)}
          aria-expanded={expanded}
          aria-label="toggle full Greeks"
        >
          {expanded ? "⌃ fewer" : "⌄ full Greeks"}
        </button>
      </div>
      {expanded && (
        <div className={styles.secondary}>
          {secondary.map((def) => (
            <GreekCell key={def.label} def={def} g={greeks} />
          ))}
        </div>
      )}
    </div>
  );
}
