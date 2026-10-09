/**
 * Stories for the Aurora design-token catalogue (tokens.css + global.css).
 *
 * This gallery makes the full token surface inspectable in one place: every
 * colour semantic, type-scale step, spacing ramp, radius, motion variable, and
 * density token is rendered as a swatch or labelled specimen.
 *
 * All colours adapt in real-time to the Storybook toolbar (Appearance / Contrast)
 * because the swatches reference CSS custom properties, not raw hex — the token
 * cascade does the rest.
 *
 * Design-system reference: GUI-DESIGN §3; tokens.css; global.css (.num, .brand-label).
 */

import type { Meta, StoryObj } from "@storybook/react";
import "./global.css";

/* -------------------------------------------------------------------------- */
/* Internal palette primitives                                                  */
/* -------------------------------------------------------------------------- */

function SectionHeading({ children }: { children: React.ReactNode }) {
  return (
    <h2
      style={{
        fontFamily: "var(--font-brand)",
        fontSize: "var(--type-headline)",
        lineHeight: "var(--type-headline-lh)",
        fontWeight: "var(--weight-header)",
        color: "var(--text-primary)",
        borderBottom: "var(--hairline)",
        paddingBottom: "var(--space-3)",
        marginBottom: "var(--space-4)",
      }}
    >
      {children}
    </h2>
  );
}

function Label({ children }: { children: React.ReactNode }) {
  return (
    <span
      className="brand-label"
      style={{
        fontSize: "var(--type-micro)",
        color: "var(--text-tertiary)",
        display: "block",
      }}
    >
      {children}
    </span>
  );
}

/* -------------------------------------------------------------------------- */
/* Colour swatches                                                              */
/* -------------------------------------------------------------------------- */

interface ColourSwatchProps {
  /** CSS custom-property name, e.g. "--brand". */
  token: string;
  /** Human-readable label shown below the swatch. */
  name: string;
}

function ColourSwatch({ token, name }: ColourSwatchProps) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-1)" }}>
      <div
        aria-label={name}
        title={token}
        style={{
          width: "100%",
          aspectRatio: "4 / 3",
          background: `var(${token})`,
          borderRadius: "var(--r-sm)",
          border: "var(--hairline)",
          boxShadow: "var(--shadow-panel)",
        }}
      />
      <span
        style={{
          fontSize: "var(--type-micro)",
          color: "var(--text-tertiary)",
          fontFamily: "var(--font-mono)",
          lineHeight: "var(--type-micro-lh)",
        }}
      >
        {name}
      </span>
    </div>
  );
}

function ColourSection({
  title,
  swatches,
}: {
  title: string;
  swatches: ColourSwatchProps[];
}) {
  return (
    <section style={{ marginBottom: "var(--space-7)" }}>
      <SectionHeading>{title}</SectionHeading>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(90px, 1fr))",
          gap: "var(--space-4)",
        }}
      >
        {swatches.map((s) => (
          <ColourSwatch key={s.token} {...s} />
        ))}
      </div>
    </section>
  );
}

/* -------------------------------------------------------------------------- */
/* Type-scale specimen                                                          */
/* -------------------------------------------------------------------------- */

const TYPE_STEPS = [
  { name: "display", size: "--type-display", lh: "--type-display-lh" },
  { name: "title", size: "--type-title", lh: "--type-title-lh" },
  { name: "headline", size: "--type-headline", lh: "--type-headline-lh" },
  { name: "body", size: "--type-body", lh: "--type-body-lh" },
  { name: "callout", size: "--type-callout", lh: "--type-callout-lh" },
  { name: "caption", size: "--type-caption", lh: "--type-caption-lh" },
  { name: "micro", size: "--type-micro", lh: "--type-micro-lh" },
] as const;

function TypeScale() {
  return (
    <section style={{ marginBottom: "var(--space-7)" }}>
      <SectionHeading>Type scale (1.20 minor third, base 13px)</SectionHeading>
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--space-4)",
        }}
      >
        {TYPE_STEPS.map(({ name, size }) => (
          <div
            key={name}
            style={{
              display: "flex",
              alignItems: "baseline",
              gap: "var(--space-4)",
              borderBottom: "var(--hairline)",
              paddingBottom: "var(--space-3)",
            }}
          >
            <Label>{name}</Label>
            <span
              style={{
                fontFamily: "var(--font-brand)",
                fontSize: `var(${size})`,
                color: "var(--text-primary)",
                flex: 1,
              }}
            >
              Celnet Aurora — {name}
            </span>
            <span
              className="num"
              style={{
                fontSize: "var(--type-micro)",
                color: "var(--text-tertiary)",
              }}
              ref={(el) => {
                if (!el) return;
                el.textContent =
                  getComputedStyle(document.documentElement)
                    .getPropertyValue(size.replace(/^var\(/, "").replace(/\)$/, ""))
                    .trim() || "—";
              }}
            />
          </div>
        ))}
      </div>
    </section>
  );
}

/* -------------------------------------------------------------------------- */
/* Spacing scale                                                                */
/* -------------------------------------------------------------------------- */

const SPACE_STEPS = [
  "--space-1",
  "--space-2",
  "--space-3",
  "--space-4",
  "--space-5",
  "--space-6",
  "--space-7",
  "--space-8",
] as const;

function SpacingScale() {
  return (
    <section style={{ marginBottom: "var(--space-7)" }}>
      <SectionHeading>Spacing scale (4px base)</SectionHeading>
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--space-2)",
        }}
      >
        {SPACE_STEPS.map((token) => (
          <div
            key={token}
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--space-4)",
            }}
          >
            <Label>{token}</Label>
            <div
              style={{
                height: 12,
                width: `var(${token})`,
                background: "var(--accent)",
                borderRadius: 2,
                flexShrink: 0,
                minWidth: 2,
              }}
            />
            <span
              className="num"
              style={{
                fontSize: "var(--type-micro)",
                color: "var(--text-tertiary)",
              }}
              ref={(el) => {
                if (!el) return;
                el.textContent =
                  getComputedStyle(document.documentElement)
                    .getPropertyValue(token)
                    .trim() || "—";
              }}
            />
          </div>
        ))}
      </div>
    </section>
  );
}

/* -------------------------------------------------------------------------- */
/* Radius + elevation                                                           */
/* -------------------------------------------------------------------------- */

function RadiusRow() {
  const radii = [
    { token: "--r-sm", label: "sm" },
    { token: "--r-md", label: "md" },
    { token: "--r-lg", label: "lg" },
    { token: "--r-full", label: "full" },
  ];

  return (
    <section style={{ marginBottom: "var(--space-7)" }}>
      <SectionHeading>Radius</SectionHeading>
      <div style={{ display: "flex", gap: "var(--space-5)", alignItems: "center", flexWrap: "wrap" }}>
        {radii.map(({ token, label }) => (
          <div
            key={token}
            style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: "var(--space-2)" }}
          >
            <div
              aria-label={label}
              style={{
                width: 56,
                height: 56,
                background: "var(--accent-soft)",
                border: "1.5px solid var(--accent)",
                borderRadius: `var(${token})`,
              }}
            />
            <Label>{label}</Label>
          </div>
        ))}
      </div>
    </section>
  );
}

function ElevationRow() {
  const elevations = [
    { token: "--shadow-panel", label: "panel" },
    { token: "--shadow-float", label: "float" },
    { token: "--shadow-hud", label: "hud" },
  ];

  return (
    <section style={{ marginBottom: "var(--space-7)" }}>
      <SectionHeading>Elevation</SectionHeading>
      <div style={{ display: "flex", gap: "var(--space-6)", alignItems: "center", flexWrap: "wrap" }}>
        {elevations.map(({ token, label }) => (
          <div
            key={token}
            style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: "var(--space-4)" }}
          >
            <div
              style={{
                width: 80,
                height: 56,
                background: "var(--bg-raised)",
                borderRadius: "var(--r-md)",
                boxShadow: `var(${token})`,
              }}
            />
            <Label>{label}</Label>
          </div>
        ))}
      </div>
    </section>
  );
}

/* -------------------------------------------------------------------------- */
/* Motion                                                                       */
/* -------------------------------------------------------------------------- */

function MotionRow() {
  const motionTokens = [
    { token: "--flash-decay", label: "flash-decay (450ms)" },
    { token: "--quote-in", label: "quote-in (180ms)" },
    { token: "--workspace-switch", label: "workspace-switch (200ms)" },
  ];

  return (
    <section style={{ marginBottom: "var(--space-7)" }}>
      <SectionHeading>Motion durations</SectionHeading>
      <div style={{ display: "flex", gap: "var(--space-4)", flexWrap: "wrap" }}>
        {motionTokens.map(({ token, label }) => (
          <div
            key={token}
            style={{
              padding: "var(--space-3) var(--space-4)",
              background: "var(--bg-inset)",
              borderRadius: "var(--r-sm)",
            }}
          >
            <Label>{label}</Label>
            <span
              className="num"
              style={{
                fontSize: "var(--type-caption)",
                color: "var(--text-primary)",
                display: "block",
                marginTop: "var(--space-1)",
              }}
              ref={(el) => {
                if (!el) return;
                el.textContent =
                  getComputedStyle(document.documentElement)
                    .getPropertyValue(token)
                    .trim() || "—";
              }}
            />
          </div>
        ))}
      </div>
    </section>
  );
}

/* -------------------------------------------------------------------------- */
/* Density tokens row                                                           */
/* -------------------------------------------------------------------------- */

function DensityTokensRow() {
  const densityTokens = [
    "--row-h",
    "--row-gap",
    "--cell-pad-x",
    "--cell-pad-y",
    "--control-h",
  ];

  return (
    <section style={{ marginBottom: "var(--space-7)" }}>
      <SectionHeading>Density tokens (live — reflect data-density on html)</SectionHeading>
      <div style={{ display: "flex", gap: "var(--space-3)", flexWrap: "wrap" }}>
        {densityTokens.map((token) => (
          <div
            key={token}
            style={{
              padding: "var(--space-3) var(--space-4)",
              background: "var(--bg-inset)",
              borderRadius: "var(--r-sm)",
              minWidth: 100,
            }}
          >
            <Label>{token}</Label>
            <span
              className="num"
              style={{
                fontSize: "var(--type-caption)",
                color: "var(--text-primary)",
                display: "block",
                marginTop: "var(--space-1)",
              }}
              ref={(el) => {
                if (!el) return;
                el.textContent =
                  getComputedStyle(document.documentElement)
                    .getPropertyValue(token)
                    .trim() || "—";
              }}
            />
          </div>
        ))}
      </div>
    </section>
  );
}

/* -------------------------------------------------------------------------- */
/* Global-css utility-class specimens                                           */
/* -------------------------------------------------------------------------- */

function UtilityClasses() {
  return (
    <section style={{ marginBottom: "var(--space-7)" }}>
      <SectionHeading>Global utility classes</SectionHeading>
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-4)" }}>
        <div>
          <Label>.num — tabular, slashed-zero numerics (SF Mono / monospace)</Label>
          <p className="num" style={{ color: "var(--text-primary)", marginTop: "var(--space-2)" }}>
            1,234,567.89 · 0.0000 · −0.1234 · ΔV 0.123
          </p>
        </div>
        <div>
          <Label>.brand-label — uppercase letter-spaced label treatment</Label>
          <p className="brand-label" style={{ color: "var(--text-secondary)", marginTop: "var(--space-2)" }}>
            EUR/USD · Tenor 3M · Strike 25Δ Call
          </p>
        </div>
        <div>
          <Label>.brand-face — display typeface (Anaheim over system-ui)</Label>
          <p
            className="brand-face"
            style={{
              fontSize: "var(--type-title)",
              color: "var(--text-primary)",
              marginTop: "var(--space-2)",
            }}
          >
            Celnet Aurora
          </p>
        </div>
      </div>
    </section>
  );
}

/* -------------------------------------------------------------------------- */
/* Full catalogue page                                                          */
/* -------------------------------------------------------------------------- */

function TokenCatalogue() {
  return (
    <div
      style={{
        padding: "var(--space-7)",
        background: "var(--bg-base)",
        minHeight: "100vh",
        fontFamily: "var(--font-ui)",
        fontSize: "var(--type-body)",
        lineHeight: "var(--type-body-lh)",
      }}
    >
      <h1
        style={{
          fontFamily: "var(--font-brand)",
          fontSize: "var(--type-display)",
          lineHeight: "var(--type-display-lh)",
          color: "var(--brand)",
          marginBottom: "var(--space-7)",
        }}
      >
        Aurora Token Catalogue
      </h1>

      <ColourSection
        title="Brand + accent"
        swatches={[
          { token: "--brand", name: "brand" },
          { token: "--brand-soft", name: "brand-soft" },
          { token: "--accent", name: "accent" },
          { token: "--accent-soft", name: "accent-soft" },
        ]}
      />

      <ColourSection
        title="Surfaces"
        swatches={[
          { token: "--bg-base", name: "bg-base" },
          { token: "--bg-raised", name: "bg-raised" },
          { token: "--bg-inset", name: "bg-inset" },
          { token: "--bg-overlay", name: "bg-overlay" },
          { token: "--bg-overlay-solid", name: "bg-overlay-solid" },
        ]}
      />

      <ColourSection
        title="Text"
        swatches={[
          { token: "--text-primary", name: "text-primary" },
          { token: "--text-secondary", name: "text-secondary" },
          { token: "--text-tertiary", name: "text-tertiary" },
        ]}
      />

      <ColourSection
        title="Market semantics"
        swatches={[
          { token: "--bid", name: "bid" },
          { token: "--offer", name: "offer" },
          { token: "--warn", name: "warn" },
          { token: "--danger", name: "danger" },
          { token: "--flash-up", name: "flash-up" },
          { token: "--flash-dn", name: "flash-dn" },
        ]}
      />

      <ColourSection
        title="Diverging heatmap ramp (indigo → neutral → green)"
        swatches={[
          { token: "--ramp-neg-2", name: "ramp-neg-2" },
          { token: "--ramp-neg-1", name: "ramp-neg-1" },
          { token: "--ramp-mid", name: "ramp-mid" },
          { token: "--ramp-pos-1", name: "ramp-pos-1" },
          { token: "--ramp-pos-2", name: "ramp-pos-2" },
        ]}
      />

      <TypeScale />
      <SpacingScale />
      <RadiusRow />
      <ElevationRow />
      <MotionRow />
      <DensityTokensRow />
      <UtilityClasses />
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Storybook meta                                                               */
/* -------------------------------------------------------------------------- */

const meta = {
  title: "Design/Tokens",
  component: TokenCatalogue,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    docs: {
      description: {
        component:
          "Full Aurora token catalogue — colours, type scale, spacing, radius, elevation, motion, and density tokens. " +
          "All colours adapt live to the Appearance / Contrast toolbar selectors because every swatch references a CSS custom property.",
      },
    },
  },
} satisfies Meta<typeof TokenCatalogue>;

export default meta;

type Story = StoryObj<typeof meta>;

/** Full catalogue — use the Appearance / Contrast toolbar to see each pole. */
export const Default: Story = {};

/** Colour-only quick reference. */
export const Colours: Story = {
  render: () => (
    <div
      style={{
        padding: "var(--space-6)",
        background: "var(--bg-base)",
        fontFamily: "var(--font-ui)",
      }}
    >
      <ColourSection
        title="Brand + accent"
        swatches={[
          { token: "--brand", name: "brand" },
          { token: "--brand-soft", name: "brand-soft" },
          { token: "--accent", name: "accent" },
          { token: "--accent-soft", name: "accent-soft" },
        ]}
      />
      <ColourSection
        title="Surfaces"
        swatches={[
          { token: "--bg-base", name: "bg-base" },
          { token: "--bg-raised", name: "bg-raised" },
          { token: "--bg-inset", name: "bg-inset" },
        ]}
      />
      <ColourSection
        title="Text"
        swatches={[
          { token: "--text-primary", name: "text-primary" },
          { token: "--text-secondary", name: "text-secondary" },
          { token: "--text-tertiary", name: "text-tertiary" },
        ]}
      />
      <ColourSection
        title="Market semantics"
        swatches={[
          { token: "--bid", name: "bid" },
          { token: "--offer", name: "offer" },
          { token: "--warn", name: "warn" },
          { token: "--danger", name: "danger" },
        ]}
      />
      <ColourSection
        title="Heatmap ramp"
        swatches={[
          { token: "--ramp-neg-2", name: "neg-2" },
          { token: "--ramp-neg-1", name: "neg-1" },
          { token: "--ramp-mid", name: "mid" },
          { token: "--ramp-pos-1", name: "pos-1" },
          { token: "--ramp-pos-2", name: "pos-2" },
        ]}
      />
    </div>
  ),
};

/** Type scale only. */
export const Typography: Story = {
  render: () => (
    <div
      style={{
        padding: "var(--space-6)",
        background: "var(--bg-base)",
        fontFamily: "var(--font-ui)",
      }}
    >
      <TypeScale />
      <UtilityClasses />
    </div>
  ),
};

/** Spacing + radius + elevation only. */
export const Geometry: Story = {
  render: () => (
    <div
      style={{
        padding: "var(--space-6)",
        background: "var(--bg-base)",
        fontFamily: "var(--font-ui)",
      }}
    >
      <SpacingScale />
      <RadiusRow />
      <ElevationRow />
    </div>
  ),
};
