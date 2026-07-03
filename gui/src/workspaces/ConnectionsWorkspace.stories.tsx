/**
 * Stories — ConnectionsWorkspace (the inbound FIX-acceptor admin surface).
 *
 * Lists every managed FIX acceptor connection the edge binds (the `FixAdminService`
 * surface) with its live `running` status and bound address, lets the operator
 * enable / disable or delete one inline, and opens the FixConnectionWizard to define
 * a new one. All state is server-owned: the hook re-fetches after each mutation so
 * the table reflects the authoritative set (persisted definitions auto-load on the
 * next edge restart).
 *
 * v1 manages Options acceptors end-to-end; SPOT FX is a later phase (the wizard
 * shows it disabled), but the table is already kind-agnostic.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { AppProvider } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { ConnectionsWorkspace } from "./ConnectionsWorkspace";

const meta = {
  title: "Workspaces/ConnectionsWorkspace",
  component: ConnectionsWorkspace,
  decorators: [
    (Story) => (
      <AppProvider transport={createMockTransport()}>
        <Story />
      </AppProvider>
    ),
  ],
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    docs: {
      description: {
        component:
          "The inbound FIX-acceptor admin surface (FixAdminService). Lists managed " +
          "acceptors with live running status + bound address, supports inline " +
          "enable/disable/delete, and opens the FixConnectionWizard to define a new " +
          "acceptor. State is server-owned; the table re-fetches after each mutation. " +
          "The FixSessionMonitor shows the live session health alongside.",
      },
    },
  },
} satisfies Meta<typeof ConnectionsWorkspace>;

export default meta;

type Story = StoryObj<typeof ConnectionsWorkspace>;

/**
 * The connections table against the mock FixAdminService. The managed-acceptor list,
 * each row's running/bound-address status, and the session monitor render live from
 * the deterministic mock; the "New connection" action opens the FixConnectionWizard.
 * Over the live edge the SAME seam drives the real acceptor lifecycle.
 */
export const Default: Story = {};
