/**
 * Stories — AdminWorkspace (user administration + desk grouping).
 *
 * Lists every user (email, name, role, desk, status) with inline edit / reset-
 * password / delete, and every desk with create / delete; a desk is the group a
 * trader belongs to, and desk membership scopes a trader's view of inbound RFQ
 * traffic. Administration is admin-only server-side, so the workspace gates on the
 * signed-in identity: anonymous or trader sessions see a sign-in / insufficient-role
 * card instead of the tables (the admin RPCs would be `permission_denied`).
 *
 * These stories exercise BOTH real states against the mock transport: the anonymous
 * gate card, and the full admin tables after signing in with the mock's seeded
 * default administrator (`admin@celnet.com` / `password`).
 */

import { useEffect } from "react";
import type { Meta, StoryObj } from "@storybook/react";
import { AppProvider, useApp } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { AdminWorkspace } from "./AdminWorkspace";

/**
 * Signs in as the mock's seeded default administrator on mount (once), so the story
 * renders the real admin surface rather than the anonymous gate card. The mock
 * seeds `admin@celnet.com` / `password`; login installs the bearer token on the
 * transport so the admin RPCs authenticate — exactly the app's own sign-in path.
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
  title: "Workspaces/AdminWorkspace",
  component: AdminWorkspace,
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
          "User administration + desk grouping (the AuthService admin surface). The " +
          "user table (inline edit / reset-password / delete) and the desk registry " +
          "(create / delete) are admin-only, so the workspace shows a sign-in / " +
          "insufficient-role card for anonymous and trader sessions and the full tables " +
          "for an administrator. State is server-owned; the hook re-fetches after each " +
          "mutation.",
      },
    },
  },
} satisfies Meta<typeof AdminWorkspace>;

export default meta;

type Story = StoryObj<typeof AdminWorkspace>;

/**
 * Anonymous — the sign-in gate. With no identity, `auth.isAdmin` is false so the
 * workspace renders its honest insufficient-role card ("Sign in with an
 * administrator account…") rather than the tables. This is the real signed-out
 * state of the mounted workspace, not a fabricated placeholder.
 */
export const Default: Story = {};

/**
 * Administrator — the full surface. Signs in with the mock's seeded default admin,
 * so the workspace renders the live user table (roles, desks, status) and the desk
 * registry with their inline actions, all fetched from the mock AuthService. The
 * same UserDialog / CapabilityMatrix affordances the live edge uses are active.
 */
export const SignedInAdmin: Story = {
  name: "Signed in as administrator",
  render: () => (
    <SignedInAsAdmin>
      <AdminWorkspace />
    </SignedInAsAdmin>
  ),
  parameters: {
    docs: {
      description: {
        story:
          "Signed in as the seeded default administrator (admin@celnet.com). The " +
          "workspace shows the authoritative user table and desk registry from the " +
          "mock AuthService, with inline edit / reset-password / delete and desk " +
          "create / delete enabled.",
      },
    },
  },
};
