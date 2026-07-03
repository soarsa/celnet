/**
 * Stories — ScopeControl (the ONE breadcrumb-driven scope control).
 *
 * The single affordance for "what slice of the firm am I looking at", whose terminal
 * case is underlier selection. It replaces the drill-up-only breadcrumb and absorbs
 * the several redundant pair affordances: there is now exactly one place to change
 * scope. Each ancestor crumb drills UP (truncates the path to it); a "drill ⌄" button
 * drills DOWN one ladder level (firm→desk→book→pair) via the scope switcher; the
 * terminal pair crumb opens the switcher's pair-universe leaf; and a group-by control
 * pins the secondary aggregation axis.
 *
 * It reads `lib/scope` through AppContext, so it renders against the mock-backed
 * AppProvider exactly as it does in the live Shell.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { AppProvider } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { ScopeControl } from "./ScopeControl";

const meta = {
  title: "Components/ScopeControl",
  component: ScopeControl,
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
          "The single breadcrumb-driven scope control. Ancestor crumbs drill up; the " +
          "drill-⌄ button drills down one ladder level (firm→desk→book→pair) via the " +
          "scope switcher; the terminal pair crumb reopens the switcher for underlier " +
          "selection; and a group-by select pins the secondary aggregation axis. Reads " +
          "scope state through AppContext.",
      },
    },
  },
} satisfies Meta<typeof ScopeControl>;

export default meta;

type Story = StoryObj<typeof ScopeControl>;

/**
 * Firm scope (the default). At the top of the ladder the breadcrumb shows the firm
 * crumb with the implied "all desks · all books" hint and a "drill desk" button;
 * changing the Group select re-pins the secondary aggregation axis. Activating the
 * drill button opens the scope switcher to pick a child node.
 */
export const Default: Story = {};
