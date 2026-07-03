/**
 * Stories — ExcelWorkspace (the "Celnet for Excel" integration page).
 *
 * A static, context-free integration page: it explains the Office.js add-in (the
 * `CELNET.*` worksheet functions + ticket task pane that bring Celnet pricing into
 * Excel over the same WS contract as this app) and offers the two get-started
 * actions — download an example workbook and download the add-in manifest to
 * sideload. The downloadable artifacts are served from the GUI's public dir under
 * `/excel/`; the deep reference is the add-in `excel/README.md`.
 *
 * The page reads no transport or AppContext state — it is wrapped in AppProvider
 * here only to mirror the other workspace stories' harness (the provider is inert
 * for this component).
 */

import type { Meta, StoryObj } from "@storybook/react";
import { AppProvider } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { ExcelWorkspace } from "./ExcelWorkspace";

const meta = {
  title: "Workspaces/ExcelWorkspace",
  component: ExcelWorkspace,
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
          "The 'Celnet for Excel' integration page. Documents the Office.js add-in — " +
          "the CELNET.* worksheet functions and ticket task pane that reach Celnet " +
          "pricing over the same WS contract as this app — and provides the download " +
          "actions for the example workbook and the sideload manifest (served from " +
          "/excel/). A purely informational surface; it holds no transport state.",
      },
    },
  },
} satisfies Meta<typeof ExcelWorkspace>;

export default meta;

type Story = StoryObj<typeof ExcelWorkspace>;

/**
 * The integration page. Renders the add-in explainer, the `CELNET.*` function
 * reference table, the default WS endpoint the add-in dials, and the two download
 * buttons (example workbook + sideload manifest). Nothing is fetched — the page is
 * a static onboarding surface for the Excel client.
 */
export const Default: Story = {};
