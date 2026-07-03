/**
 * Stories — ReferenceDataWorkspace (the instrument reference-data registry).
 *
 * Lists every instrument DEFINITION (its family, currency, conventions and external
 * identifiers) for ANY authenticated user; create / edit / delete are administrator-
 * only (server-enforced), so those controls are gated PER-CONTROL — every signed-in
 * user sees the list, only an admin sees the form and the row actions.
 *
 * Unlike the Administration workspace this is NOT hard-gated to admins, but it does
 * require a signed-in identity to read the registry: an anonymous session sees the
 * sign-in card. These stories exercise both real states against the mock — the
 * anonymous gate and the populated registry after signing in as the seeded admin.
 */

import { useEffect } from "react";
import type { Meta, StoryObj } from "@storybook/react";
import { AppProvider, useApp } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { ReferenceDataWorkspace } from "./ReferenceDataWorkspace";

/**
 * Signs in as the mock's seeded default administrator on mount (once) so the story
 * renders the populated registry (and the admin-only create/edit/delete controls)
 * rather than the anonymous sign-in card (`admin@celnet.com` / `password`).
 */
function SignedInAsAdmin({ children }: { children: React.ReactNode }): React.ReactElement {
  const { auth } = useApp();
  const { login, user } = auth;
  useEffect(() => {
    if (!user) void login("admin@celnet.com", "password").catch(() => undefined);
  }, [login, user]);
  return <>{children}</>;
}

const meta = {
  title: "Workspaces/ReferenceDataWorkspace",
  component: ReferenceDataWorkspace,
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
          "The instrument reference-data registry (the AuthService instrument surface). " +
          "Every authenticated user reads the instrument-definition list (family, " +
          "currency, conventions, external identifiers); create / edit / delete are " +
          "admin-only and gated per-control. Anonymous sessions see the sign-in card. " +
          "State is server-owned; the hook re-fetches after each mutation.",
      },
    },
  },
} satisfies Meta<typeof ReferenceDataWorkspace>;

export default meta;

type Story = StoryObj<typeof ReferenceDataWorkspace>;

/**
 * Anonymous — the sign-in gate. Without an identity the workspace renders the honest
 * "sign in to read the registry" card rather than the list. The real signed-out
 * state of the mounted workspace.
 */
export const Default: Story = {};

/**
 * Administrator — the populated registry with admin controls. Signs in with the
 * mock's seeded default admin so the workspace renders the instrument-definition
 * table (family / currency / conventions / external-id chips) plus the admin-only
 * create form and per-row edit / delete actions, all against the mock AuthService.
 */
export const SignedInAdmin: Story = {
  name: "Signed in as administrator",
  render: () => (
    <SignedInAsAdmin>
      <ReferenceDataWorkspace />
    </SignedInAsAdmin>
  ),
  parameters: {
    docs: {
      description: {
        story:
          "Signed in as the seeded default administrator (admin@celnet.com). The " +
          "registry table lists every instrument definition; the admin-only create " +
          "form and per-row edit / delete controls are enabled (a trader identity " +
          "would see the same list read-only).",
      },
    },
  },
};
