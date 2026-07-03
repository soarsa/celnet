/**
 * Stories — CommandPalette (⌘K, the universal escape hatch + keyboard-first spine).
 *
 * Fuzzy over pairs, workspaces, and actions — no function codes, no <GO>. Rendered
 * on the `hud` material (thick blur); it owns focus while open, is fully keyboard-
 * driven (↑/↓ to move, Enter to run, Escape to close), and honours a fuzzy query.
 * Pure and context-free: it takes `open`, a `commands` list, and `onClose`, so the
 * stories drive it directly with a synthetic command set.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { CommandPalette, type Command } from "./CommandPalette";

/** A representative command set spanning the three groups the palette fuzzes over. */
const COMMANDS: Command[] = [
  { id: "eurusd", title: "EUR/USD", hint: "European euro / US dollar", group: "Pairs", run: () => {} },
  { id: "gbpusd", title: "GBP/USD", hint: "Sterling / US dollar", group: "Pairs", run: () => {} },
  { id: "usdjpy", title: "USD/JPY", hint: "US dollar / Japanese yen", group: "Pairs", run: () => {} },
  { id: "ws-stream", title: "Stream", hint: "RFS blotter", group: "Workspaces", run: () => {} },
  { id: "ws-risk", title: "Risk", hint: "Scenario grid", group: "Workspaces", run: () => {} },
  { id: "ws-book", title: "Book", hint: "Firm risk", group: "Workspaces", run: () => {} },
  { id: "ws-surface", title: "Surface", hint: "Mark vol surface", group: "Workspaces", run: () => {} },
  { id: "act-newticket", title: "New ticket", hint: "Open the deal ticket", group: "Actions", run: () => {} },
  { id: "act-signout", title: "Sign out", hint: "End the session", group: "Actions", run: () => {} },
];

const meta = {
  title: "Components/CommandPalette",
  component: CommandPalette,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    docs: {
      description: {
        component:
          "The ⌘K command palette — fuzzy over pairs, workspaces, and actions on the " +
          "hud material. Fully keyboard-driven (↑/↓ + Enter + Escape). Pure: takes " +
          "open / commands / onClose, so it drives directly from a command list with " +
          "no transport or context.",
      },
    },
  },
  args: {
    open: true,
    commands: COMMANDS,
    onClose: () => {},
  },
} satisfies Meta<typeof CommandPalette>;

export default meta;

type Story = StoryObj<typeof CommandPalette>;

/**
 * Open with the full command set. Type into the search box to fuzzy-filter across
 * the pair / workspace / action groups; ↑/↓ moves the active row, Enter runs it, and
 * Escape closes (calling `onClose`). The top nine ranked matches are shown.
 */
export const Default: Story = {};

/**
 * Closed — `open` is false, so the palette renders nothing (returns null). This is
 * the resting state: the palette only mounts its overlay while open. Toggle the
 * `open` control to reveal it.
 */
export const Closed: Story = {
  args: { open: false },
  parameters: {
    docs: {
      description: {
        story:
          "With open=false the component returns null (no overlay). Flip the open " +
          "control in Controls to see it mount and grab focus.",
      },
    },
  },
};
