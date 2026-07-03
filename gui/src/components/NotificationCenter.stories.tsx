/**
 * Stories — NotificationCenter (the signed-in-only desk notification surface).
 *
 * Opens the dedicated `streamNotifications` push channel once a user is signed in and
 * renders TWO affordances from the same feed: transient TOASTS for new
 * RFQ/IOI-requires-pricing events (auto-dismissed, announced via an aria-live region),
 * and a persistent BELL + unread badge whose dropdown lists recent events (clicking an
 * item routes to the Quoting workspace). One contract, two transports: the feed is the
 * CelnetTransport.streamNotifications seam, identical across the in-app mock and the
 * live NotificationService edge.
 *
 * It renders NOTHING when signed out (returns null), so the meaningful story signs in
 * with the mock's seeded default administrator to mount the bell.
 */

import { useEffect } from "react";
import type { Meta, StoryObj } from "@storybook/react";
import { AppProvider, useApp } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { NotificationCenter } from "./NotificationCenter";

/**
 * Signs in as the mock's seeded default administrator on mount (once) so the
 * signed-in-only NotificationCenter mounts its bell (`admin@celnet.com` /
 * `password`) — the app's own sign-in path installs the bearer token and opens the
 * push channel.
 */
function SignedIn({ children }: { children: React.ReactNode }): React.ReactElement {
  const { auth } = useApp();
  const { login, user } = auth;
  useEffect(() => {
    if (!user) void login("admin@celnet.com", "password").catch(() => undefined);
  }, [login, user]);
  return <>{children}</>;
}

const meta = {
  title: "Components/NotificationCenter",
  component: NotificationCenter,
  decorators: [
    (Story) => (
      <AppProvider transport={createMockTransport()}>
        <Story />
      </AppProvider>
    ),
  ],
  tags: ["autodocs"],
  parameters: {
    layout: "padded",
    docs: {
      description: {
        component:
          "The signed-in-only desk notification surface: transient toasts for new " +
          "RFQ/IOI events (aria-live announced) plus a persistent bell + unread badge " +
          "whose dropdown routes to Quoting. Driven by the CelnetTransport " +
          "streamNotifications seam. Returns null when signed out.",
      },
    },
  },
} satisfies Meta<typeof NotificationCenter>;

export default meta;

type Story = StoryObj<typeof NotificationCenter>;

/**
 * Signed in — the bell. Signing in as the seeded admin opens the notification push
 * channel and mounts the persistent bell affordance (no unread until a desk request
 * arrives — the mock only emits on real desk-request submission, never fabricated).
 * Clicking the bell opens the recent-events dropdown.
 *
 * Note: signed out, this component renders null by design (it is a signed-in-only
 * surface) — hence the story establishes an identity rather than showing a blank.
 */
export const Default: Story = {
  name: "Signed in (bell mounted)",
  render: () => (
    <SignedIn>
      <NotificationCenter />
    </SignedIn>
  ),
};
