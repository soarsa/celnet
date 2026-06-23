/**
 * Panel stories — the workspace material primitive (GUI-DESIGN §3.3). Demonstrates
 * the three materials (panel / float / hud), the optional title+glyph+actions
 * header, scrollable-region keyboard accessibility, and the titleless variant.
 * All spacing and surface tokens come from the Aurora cascade; no raw hex.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { Panel } from "./Panel";
import { Button } from "./Button";

const MATERIALS = ["panel", "float", "hud"] as const;

const meta = {
  title: "Components/Panel",
  component: Panel,
  tags: ["autodocs"],
  parameters: {
    // Panels need breathing room — override the centered layout for a wider canvas.
    layout: "padded",
  },
  argTypes: {
    material: {
      control: "inline-radio",
      options: MATERIALS,
      description:
        "panel = bg-raised hairline shadow (default); float = translucent overlay + blur; hud = thick overlay (command palette / inspector).",
    },
    title: { control: "text" },
    glyph: { control: "text" },
    noPadding: { control: "boolean" },
  },
  args: {
    material: "panel",
    title: "Panel title",
    children: (
      <p style={{ color: "var(--text-secondary)", fontSize: "var(--type-body)", margin: 0 }}>
        Panel body content. Replace with any workspace child.
      </p>
    ),
  },
} satisfies Meta<typeof Panel>;

export default meta;

type Story = StoryObj<typeof meta>;

/** Default panel with title — the most common workspace tile. */
export const Default: Story = {};

/** Glyph prefix in the title bar. */
export const WithGlyph: Story = {
  args: { glyph: "◈", title: "Greeks" },
};

/** Actions slot — right-justified controls in the header. */
export const WithActions: Story = {
  args: {
    glyph: "≡",
    title: "Positions",
    actions: (
      <Button variant="ghost" size="md">
        Export
      </Button>
    ),
  },
};

/** Float material — translucent overlay, e.g. a tooltip panel or drawer. */
export const Float: Story = {
  args: {
    material: "float",
    title: "Market context",
    glyph: "◦",
  },
};

/** HUD material — thick blur, used for the command palette and inspector. */
export const Hud: Story = {
  args: {
    material: "hud",
    title: "Inspector",
    glyph: "⊕",
  },
};

/** No title — the panel renders without a header bar (edge-to-edge body). */
export const NoTitle: Story = {
  args: {
    title: undefined,
    glyph: undefined,
    children: (
      <p style={{ color: "var(--text-secondary)", fontSize: "var(--type-body)", margin: 0 }}>
        Titleless panel — body is the full surface.
      </p>
    ),
  },
};

/**
 * Scrollable body — content exceeds the panel height so the body auto-acquires
 * tabIndex=0 and role="region" (WCAG 2.1.1 / axe scrollable-region-focusable).
 * Resize the canvas to a short height to see the overflow kick in.
 */
export const Scrollable: Story = {
  args: {
    title: "Blotter",
    glyph: "≡",
    children: (
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
        {Array.from({ length: 30 }, (_, i) => (
          <div
            key={i}
            style={{
              padding: "var(--space-2) var(--space-3)",
              background: "var(--bg-inset)",
              borderRadius: "var(--r-sm)",
              fontSize: "var(--type-body)",
              fontFamily: "var(--font-mono)",
              color: "var(--text-primary)",
            }}
          >
            Row {i + 1} — EUR/USD 1.08{(540 + i).toString().padStart(3, "0")}
          </div>
        ))}
      </div>
    ),
  },
};

/** All three materials side by side — shows the depth hierarchy at a glance. */
export const MaterialPalette: Story = {
  render: () => (
    <div
      style={{
        display: "flex",
        gap: "var(--space-6)",
        flexWrap: "wrap",
        padding: "var(--space-6)",
        background: "var(--bg-base)",
        borderRadius: "var(--r-md)",
      }}
    >
      {MATERIALS.map((m) => (
        <Panel key={m} material={m} title={m} glyph="○" style={{ width: 200 } as React.CSSProperties}>
          <p
            style={{
              color: "var(--text-secondary)",
              fontSize: "var(--type-caption)",
              margin: 0,
            }}
          >
            {m} surface
          </p>
        </Panel>
      ))}
    </div>
  ),
};
