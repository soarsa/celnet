/**
 * Stories for the Aurora density subsystem (density.ts).
 *
 * Density is the third orthogonal token axis (alongside Appearance and Contrast).
 * It is a data-attribute on <html> (`data-density = comfortable | compact`);
 * the token cascade rewrites --row-h / --cell-pad-x / --cell-pad-y / --row-gap /
 * --control-h for every component on screen without any JS per-component.
 *
 * `comfortable` (default) keeps the relaxed dense-pro rhythm.
 * `compact` collapses metrics for IB-cardinality grids (thousands of rows/screen).
 *
 * Design-system reference: GUI-DESIGN §3 (density axis); density.ts; tokens.css
 * `:root[data-density="compact"]` block.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { useDensity, applyDensity } from "./density";
import type { Density } from "./density";
import "./global.css";

/* -------------------------------------------------------------------------- */
/* Demo component                                                               */
/* -------------------------------------------------------------------------- */

interface DensityDemoProps {
  /** Initial density — the hook picks this up on mount via localStorage. */
  initialDensity: Density;
}

function DensityDemo({ initialDensity }: DensityDemoProps) {
  try {
    localStorage.setItem("celnet.density", initialDensity);
  } catch {
    /* storage unavailable; hook falls back gracefully */
  }

  const { density, setDensity, toggleDensity } = useDensity();

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--space-5)",
        padding: "var(--space-6)",
        background: "var(--bg-raised)",
        borderRadius: "var(--r-md)",
        boxShadow: "var(--shadow-panel)",
        minWidth: 380,
        color: "var(--text-primary)",
        fontFamily: "var(--font-ui)",
        fontSize: "var(--type-body)",
        lineHeight: "var(--type-body-lh)",
      }}
    >
      {/* Active density label */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <span
          className="brand-label"
          style={{ fontSize: "var(--type-caption)", color: "var(--text-tertiary)" }}
        >
          density axis
        </span>
        <span
          style={{
            fontFamily: "var(--font-mono)",
            fontSize: "var(--type-callout)",
            color: "var(--brand)",
          }}
        >
          {density}
        </span>
      </div>

      {/* Toggle buttons */}
      <div style={{ display: "flex", gap: "var(--space-3)" }}>
        {(["comfortable", "compact"] as Density[]).map((d) => (
          <button
            key={d}
            type="button"
            onClick={() => setDensity(d)}
            style={{
              flex: 1,
              padding: "var(--space-2) var(--space-3)",
              background:
                density === d ? "var(--accent-soft)" : "var(--bg-inset)",
              color: density === d ? "var(--accent)" : "var(--text-secondary)",
              borderRadius: "var(--r-sm)",
              border: density === d
                ? "1px solid var(--accent)"
                : "1px solid transparent",
              fontSize: "var(--type-callout)",
              fontFamily: "var(--font-ui)",
              cursor: "pointer",
              transition: `background var(--quote-in) var(--ease-out)`,
            }}
          >
            {d}
          </button>
        ))}
        <button
          type="button"
          onClick={toggleDensity}
          style={{
            padding: "var(--space-2) var(--space-3)",
            background: "var(--brand-soft)",
            color: "var(--brand)",
            borderRadius: "var(--r-sm)",
            border: "none",
            fontSize: "var(--type-callout)",
            fontFamily: "var(--font-ui)",
            cursor: "pointer",
          }}
        >
          toggle
        </button>
      </div>

      {/* Token readout — shows the live CSS custom-property values */}
      <TokenReadout />

      {/* Simulated blotter rows — height comes exclusively from --row-h */}
      <BlotterPreview rowCount={6} />
    </div>
  );
}

/** Reads live CSS custom-property values from the root and renders them. */
function TokenReadout() {
  const tokens: { name: string; token: string }[] = [
    { name: "--row-h", token: "--row-h" },
    { name: "--row-gap", token: "--row-gap" },
    { name: "--cell-pad-x", token: "--cell-pad-x" },
    { name: "--cell-pad-y", token: "--cell-pad-y" },
    { name: "--control-h", token: "--control-h" },
  ];

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--space-1)",
      }}
    >
      <span
        className="brand-label"
        style={{ fontSize: "var(--type-micro)", color: "var(--text-tertiary)", marginBottom: "var(--space-1)" }}
      >
        density tokens (live)
      </span>
      {tokens.map(({ name, token }) => (
        <div
          key={name}
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            padding: "var(--cell-pad-y) var(--cell-pad-x)",
            background: "var(--bg-inset)",
            borderRadius: "var(--r-sm)",
            minHeight: "var(--row-h)",
            transition: `min-height var(--workspace-switch) var(--ease-out), padding var(--workspace-switch) var(--ease-out)`,
          }}
        >
          <code
            style={{
              fontFamily: "var(--font-mono)",
              fontSize: "var(--type-caption)",
              color: "var(--text-secondary)",
            }}
          >
            {name}
          </code>
          {/* The actual resolved value is set via inline CSS var reference so the
           * cascade resolves it. The displayed text reads the computed style. */}
          <span
            ref={(el) => {
              if (!el) return;
              const v = getComputedStyle(document.documentElement)
                .getPropertyValue(`--${token.replace(/^--/, "")}`)
                .trim();
              el.textContent = v || "—";
            }}
            style={{
              fontFamily: "var(--font-mono)",
              fontSize: "var(--type-caption)",
              color: "var(--text-primary)",
            }}
          />
        </div>
      ))}
    </div>
  );
}

interface BlotterPreviewProps {
  rowCount: number;
}

const INSTRUMENTS = [
  "EUR/USD 1M 25Δ Call",
  "GBP/USD 3M ATM Straddle",
  "USD/JPY 1W RR",
  "EUR/GBP 2M Fly",
  "AUD/USD 6M ATM",
  "USD/CHF 1Y 10Δ Put",
];

/** A simulated price-blotter whose row height tracks --row-h token changes. */
function BlotterPreview({ rowCount }: BlotterPreviewProps) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--row-gap)",
        border: "var(--hairline)",
        borderRadius: "var(--r-sm)",
        overflow: "hidden",
      }}
    >
      {/* Header */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 80px 80px 64px",
          gap: 0,
          padding: "var(--cell-pad-y) var(--cell-pad-x)",
          background: "var(--bg-base)",
          borderBottom: "var(--hairline)",
        }}
      >
        {["Instrument", "Bid", "Offer", "Health"].map((h) => (
          <span
            key={h}
            className="brand-label"
            style={{
              fontSize: "var(--type-micro)",
              color: "var(--text-tertiary)",
            }}
          >
            {h}
          </span>
        ))}
      </div>

      {/* Rows */}
      {INSTRUMENTS.slice(0, rowCount).map((inst, i) => (
        <div
          key={inst}
          style={{
            display: "grid",
            gridTemplateColumns: "1fr 80px 80px 64px",
            gap: 0,
            padding: "var(--cell-pad-y) var(--cell-pad-x)",
            minHeight: "var(--row-h)",
            alignItems: "center",
            background: i % 2 === 0 ? "var(--bg-raised)" : "var(--bg-base)",
            transition: `min-height var(--workspace-switch) var(--ease-out), padding var(--workspace-switch) var(--ease-out)`,
          }}
        >
          <span
            style={{
              fontSize: "var(--type-body)",
              color: "var(--text-primary)",
            }}
          >
            {inst}
          </span>
          <span
            className="num"
            style={{ color: "var(--bid)", fontSize: "var(--type-body)" }}
          >
            {(1.0852 + i * 0.0013).toFixed(4)}
          </span>
          <span
            className="num"
            style={{ color: "var(--offer)", fontSize: "var(--type-body)" }}
          >
            {(1.0854 + i * 0.0013).toFixed(4)}
          </span>
          <span style={{ color: "var(--bid)", fontSize: "var(--type-caption)" }}>◉</span>
        </div>
      ))}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Storybook meta                                                               */
/* -------------------------------------------------------------------------- */

const DENSITIES: Density[] = ["comfortable", "compact"];

const meta = {
  title: "Design/Density",
  component: DensityDemo,
  tags: ["autodocs"],
  argTypes: {
    initialDensity: {
      control: "inline-radio",
      options: DENSITIES,
      description:
        "Starting density. The hook persists to localStorage; toggle buttons and this Control both drive the cascade through data-density on <html>.",
    },
  },
  args: {
    initialDensity: "comfortable",
  },
} satisfies Meta<typeof DensityDemo>;

export default meta;

type Story = StoryObj<typeof meta>;

/** Default comfortable density — relaxed dense-pro rhythm. */
export const Default: Story = {};

export const Comfortable: Story = {
  args: { initialDensity: "comfortable" },
};

/** Compact density — IB-cardinality blotter mode (row height drops to 26px). */
export const Compact: Story = {
  args: { initialDensity: "compact" },
};

/**
 * Side-by-side blotters at both densities. The `applyDensity` utility is used
 * here to stamp the attribute without the hook (it is the hook's imperative
 * escape hatch for non-React contexts and tests).
 *
 * Note: because the two panels live in the same cascade, only one data-density
 * attribute can be active on <html> at a time. The comparison panels are labelled
 * to reflect the design intent; use the Controls to pick which pole is live.
 */
export const SideBySide: Story = {
  render: () => {
    // Drive the cascade into comfortable for this story's context
    applyDensity("comfortable");
    return (
      <div style={{ display: "flex", gap: "var(--space-5)", alignItems: "flex-start", flexWrap: "wrap" }}>
        <div>
          <span
            className="brand-label"
            style={{
              display: "block",
              fontSize: "var(--type-micro)",
              color: "var(--text-tertiary)",
              marginBottom: "var(--space-2)",
            }}
          >
            comfortable (default)
          </span>
          <BlotterPreview rowCount={4} />
        </div>
        <div>
          <span
            className="brand-label"
            style={{
              display: "block",
              fontSize: "var(--type-micro)",
              color: "var(--text-tertiary)",
              marginBottom: "var(--space-2)",
            }}
          >
            compact (IB-cardinality)
          </span>
          {/* Compact blotter: same component, data-density attribute drives it */}
          <BlotterPreview rowCount={4} />
        </div>
      </div>
    );
  },
};
