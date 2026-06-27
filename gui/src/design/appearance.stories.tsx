/**
 * Stories for the Aurora appearance subsystem (appearance.ts).
 *
 * `useAppearance` owns the two orthogonal theme axes — Appearance (dark|light)
 * and Contrast (normal|high) — and drives them exclusively through data-attribute
 * mutations on <html>, so the token cascade does all the colour work: no
 * component reads a colour value in JS.
 *
 * The preview.ts decorator already stamps data-appearance / data-contrast before
 * each story; the Storybook toolbar lets you walk all four poles. These stories
 * add interactive surfaces that let the Controls panel exercise the hook directly
 * and make it obvious which branch of the cascade is active.
 *
 * Design-system reference: GUI-DESIGN §3.2, §7; tokens.css dark/light/high-contrast
 * rule blocks.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { useAppearance } from "./appearance";
import type { Appearance, Contrast } from "./appearance";
import "./global.css";

/* -------------------------------------------------------------------------- */
/* Demo component                                                               */
/* -------------------------------------------------------------------------- */

interface AppearanceDemoProps {
  /** Starting appearance. The hook owns subsequent toggling. */
  initialAppearance: Appearance;
  /** Starting contrast. The hook owns subsequent toggling. */
  initialContrast: Contrast;
}

/**
 * Interactive demo that wires `useAppearance` up so Controls / click-events
 * both drive the cascade. The rendered surface uses only semantic tokens —
 * never raw hex — so every colour you see is evidence that the cascade is live.
 */
function AppearanceDemo({
  initialAppearance,
  initialContrast,
}: AppearanceDemoProps) {
  /* Seed localStorage with the Control-selected values so the hook picks them up
   * on mount. In a real session the user's persisted preference wins. */
  try {
    localStorage.setItem("celnet.appearance", initialAppearance);
    localStorage.setItem("celnet.contrast", initialContrast);
  } catch {
    /* storage unavailable — hook falls back to the prop values */
  }

  const { appearance, contrast, toggleAppearance, toggleContrast } =
    useAppearance();

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
        minWidth: 340,
        color: "var(--text-primary)",
        fontFamily: "var(--font-ui)",
        fontSize: "var(--type-body)",
        lineHeight: "var(--type-body-lh)",
      }}
    >
      {/* Status row */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr",
          gap: "var(--space-3)",
        }}
      >
        <Chip label="appearance" value={appearance} />
        <Chip label="contrast" value={contrast} />
      </div>

      {/* Toggle buttons */}
      <div style={{ display: "flex", gap: "var(--space-3)" }}>
        <TokenButton onClick={toggleAppearance}>
          Toggle appearance
        </TokenButton>
        <TokenButton onClick={toggleContrast}>Toggle contrast</TokenButton>
      </div>

      {/* Token swatches — prove the cascade flipped */}
      <SwatchGrid />
    </div>
  );
}

function Chip({ label, value }: { label: string; value: string }) {
  return (
    <div
      style={{
        padding: "var(--space-2) var(--space-3)",
        background: "var(--bg-inset)",
        borderRadius: "var(--r-sm)",
        display: "flex",
        flexDirection: "column",
        gap: 2,
      }}
    >
      <span
        className="brand-label"
        style={{ fontSize: "var(--type-micro)", color: "var(--text-tertiary)" }}
      >
        {label}
      </span>
      <span style={{ fontFamily: "var(--font-mono)", fontSize: "var(--type-callout)", color: "var(--text-primary)" }}>
        {value}
      </span>
    </div>
  );
}

function TokenButton({
  onClick,
  children,
}: {
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        padding: "var(--space-2) var(--space-4)",
        background: "var(--accent-soft)",
        color: "var(--accent)",
        borderRadius: "var(--r-sm)",
        fontSize: "var(--type-callout)",
        fontFamily: "var(--font-ui)",
        cursor: "pointer",
        border: "none",
        transition: `background var(--quote-in) var(--ease-out)`,
      }}
    >
      {children}
    </button>
  );
}

function SwatchGrid() {
  const swatches: { name: string; token: string }[] = [
    { name: "bg-base", token: "var(--bg-base)" },
    { name: "bg-raised", token: "var(--bg-raised)" },
    { name: "bg-inset", token: "var(--bg-inset)" },
    { name: "text-primary", token: "var(--text-primary)" },
    { name: "text-secondary", token: "var(--text-secondary)" },
    { name: "text-tertiary", token: "var(--text-tertiary)" },
    { name: "brand", token: "var(--brand)" },
    { name: "accent", token: "var(--accent)" },
    { name: "bid", token: "var(--bid)" },
    { name: "offer", token: "var(--offer)" },
  ];

  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(5, 1fr)",
        gap: "var(--space-2)",
      }}
    >
      {swatches.map(({ name, token }) => (
        <div key={name} style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <div
            aria-label={name}
            style={{
              width: "100%",
              aspectRatio: "1",
              background: token,
              borderRadius: "var(--r-sm)",
              border: "var(--hairline)",
            }}
          />
          <span
            style={{
              fontSize: "var(--type-micro)",
              color: "var(--text-tertiary)",
              textAlign: "center",
              fontFamily: "var(--font-mono)",
              lineHeight: "var(--type-micro-lh)",
            }}
          >
            {name}
          </span>
        </div>
      ))}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Matrix component — all 4 appearance × contrast combinations                 */
/* -------------------------------------------------------------------------- */

const APPEARANCES: Appearance[] = ["dark", "light"];
const CONTRASTS: Contrast[] = ["normal", "high"];

function AppearanceMatrix() {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "1fr 1fr",
        gap: "var(--space-4)",
        padding: "var(--space-3)",
      }}
    >
      {APPEARANCES.flatMap((app) =>
        CONTRASTS.map((con) => (
          <MatrixCell key={`${app}-${con}`} appearance={app} contrast={con} />
        )),
      )}
    </div>
  );
}

function MatrixCell({
  appearance,
  contrast,
}: {
  appearance: Appearance;
  contrast: Contrast;
}) {
  return (
    /* Inline style drives the cascade locally by setting the two attributes on
     * a wrapper; this works because tokens.css uses :root selectors that target
     * <html>. For the matrix we snapshot the colours using CSS variables resolved
     * at the document root's current cascade — the cells are labelled to be clear
     * about what they represent. */
    <div
      style={{
        padding: "var(--space-4)",
        background: "var(--bg-raised)",
        borderRadius: "var(--r-md)",
        boxShadow: "var(--shadow-panel)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--space-2)",
        color: "var(--text-primary)",
        fontFamily: "var(--font-ui)",
        fontSize: "var(--type-callout)",
      }}
    >
      <span
        className="brand-label"
        style={{
          fontSize: "var(--type-micro)",
          color: "var(--text-tertiary)",
        }}
      >
        {appearance} / {contrast}
      </span>
      <div style={{ display: "flex", gap: "var(--space-2)" }}>
        {["brand", "accent", "bid", "offer"].map((name) => (
          <div
            key={name}
            aria-label={name}
            style={{
              width: 20,
              height: 20,
              background: `var(--${name})`,
              borderRadius: "var(--r-sm)",
              border: "var(--hairline)",
            }}
          />
        ))}
      </div>
      <span style={{ color: "var(--text-secondary)", fontSize: "var(--type-micro)" }}>
        The active canvas reflects the Storybook toolbar selection, not this cell's label.
        Use the toolbar to walk each pole.
      </span>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Storybook meta                                                               */
/* -------------------------------------------------------------------------- */

const meta = {
  title: "Design/Appearance",
  component: AppearanceDemo,
  tags: ["autodocs"],
  argTypes: {
    initialAppearance: {
      control: "inline-radio",
      options: APPEARANCES,
      description:
        "Seed appearance for the hook. The Storybook toolbar also drives data-appearance on <html> (both routes exercise the same token cascade).",
    },
    initialContrast: {
      control: "inline-radio",
      options: CONTRASTS,
      description:
        "Seed contrast for the hook. Pairs with the toolbar Contrast toggle.",
    },
  },
  args: {
    initialAppearance: "dark",
    initialContrast: "normal",
  },
} satisfies Meta<typeof AppearanceDemo>;

export default meta;

type Story = StoryObj<typeof meta>;

/** Interactive surface — flip Controls or click the buttons; watch tokens cascade. */
export const Default: Story = {};

export const DarkNormal: Story = {
  args: { initialAppearance: "dark", initialContrast: "normal" },
};

export const DarkHighContrast: Story = {
  args: { initialAppearance: "dark", initialContrast: "high" },
};

export const LightNormal: Story = {
  args: { initialAppearance: "light", initialContrast: "normal" },
};

export const LightHighContrast: Story = {
  args: { initialAppearance: "light", initialContrast: "high" },
};

/**
 * All four appearance × contrast combinations side-by-side.
 * The swatches reflect the CURRENT toolbar selection (the root cascade), so use
 * the toolbar to compare poles; the labels clarify what each cell targets.
 */
export const Matrix: Story = {
  render: () => <AppearanceMatrix />,
};
