/**
 * Stories — PermissionsWorkspace (the component-access administration page).
 *
 * A standalone rail workspace (not a panel buried behind the Admin table): a left
 * user picker, and on the right the ComponentAccessGrid for the selected user
 * (Read/Write toggle widgets per component, with an advanced per-capability
 * disclosure). Editing is a per-user overlay on top of the server's role model.
 *
 * Admin-only, server- and client-side: it renders the SAME sign-in / insufficient-
 * role card the Admin workspace uses when `!auth.isAdmin` (the underlying capability
 * RPCs would be `permission_denied`). These stories exercise both real states — the
 * anonymous gate and the full grid after signing in as the mock's seeded admin.
 */

import { useEffect } from "react";
import type { Meta, StoryObj } from "@storybook/react";
import { AppProvider, useApp } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { PermissionsWorkspace } from "./PermissionsWorkspace";

/**
 * Signs in as the mock's seeded default administrator on mount (once) so the story
 * renders the real permissions surface rather than the anonymous gate card
 * (`admin@celnet.com` / `password`) — the app's own sign-in path.
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
  title: "Workspaces/PermissionsWorkspace",
  component: PermissionsWorkspace,
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
          "The first-class component-access administration page. A left user picker + " +
          "the ComponentAccessGrid for the selected user (per-component Read/Write with " +
          "an advanced per-capability disclosure). Admin-only: anonymous / trader " +
          "sessions see the insufficient-role card; an administrator sees the roster " +
          "and the editable access grid. Edits overlay the server's role model.",
      },
    },
  },
} satisfies Meta<typeof PermissionsWorkspace>;

export default meta;

type Story = StoryObj<typeof PermissionsWorkspace>;

/**
 * Anonymous — the sign-in gate. With no identity, `auth.isAdmin` is false, so the
 * workspace renders the same honest insufficient-role card the Admin workspace uses.
 * The real signed-out state, not a placeholder.
 */
export const Default: Story = {};

/**
 * Administrator — the full permissions surface. Signs in with the mock's seeded
 * default admin so the workspace renders the live user roster and the
 * ComponentAccessGrid for the selected user, editable against the mock AuthService.
 */
export const SignedInAdmin: Story = {
  name: "Signed in as administrator",
  render: () => (
    <SignedInAsAdmin>
      <PermissionsWorkspace />
    </SignedInAsAdmin>
  ),
  parameters: {
    docs: {
      description: {
        story:
          "Signed in as the seeded default administrator (admin@celnet.com). The user " +
          "picker lists the roster from the mock AuthService; selecting a user opens " +
          "the ComponentAccessGrid with per-component Read/Write toggles and the " +
          "advanced per-capability disclosure.",
      },
    },
  },
};
