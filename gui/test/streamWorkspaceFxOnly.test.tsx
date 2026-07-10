/**
 * StreamWorkspace — FX-only render guard.
 *
 * The FX Options Stream workspace enforces hard vertical asset separation: it
 * shows the FX two-way streaming blotter ONLY. The embedded fixed-income strip
 * (OIS/IRS/FRA/bond PV + PV01/DV01 lines) was removed once Fixed Income gained
 * its own dedicated FI Streaming hub (FiStreamingWorkspace). This test pins that
 * separation: the FX grid region renders, and no fixed-income section survives.
 */

import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { StreamWorkspace } from "../src/workspaces/StreamWorkspace";

beforeEach(() => {
  window.history.replaceState(null, "", "/?mock");
});
afterEach(() => {
  window.history.replaceState(null, "", "/");
});

async function renderStream(): Promise<void> {
  await act(async () => {
    render(
      <AppProvider>
        <StreamWorkspace />
      </AppProvider>,
    );
  });
}

describe("StreamWorkspace — FX-only, no embedded fixed-income strip", () => {
  it("renders the FX streaming two-way blotter region and its controls", async () => {
    await renderStream();
    // The FX two-way grid region is the workspace's subject.
    expect(
      await screen.findByRole("region", { name: /streaming two-way markets/i }),
    ).toBeInTheDocument();
    // The FX scale controls (group-by + column toggles) are present.
    expect(screen.getByRole("group", { name: /group by/i })).toBeInTheDocument();
    expect(screen.getByRole("group", { name: /columns/i })).toBeInTheDocument();
    // The FX foot note stays.
    expect(screen.getByText(/click a side to trade/i)).toBeInTheDocument();
  });

  it("shows NO fixed-income / rates streaming section", async () => {
    await renderStream();
    await screen.findByRole("region", { name: /streaming two-way markets/i });
    // The removed FI strip: title, desk note, and its rates instrument chips/columns.
    expect(
      screen.queryByRole("region", { name: /streaming fixed-income lines/i }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/fixed income/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/rates route to/i)).not.toBeInTheDocument();
    // No rates instrument chips.
    expect(screen.queryByText("OIS 2Y rec")).not.toBeInTheDocument();
    expect(screen.queryByText("IRS 5Y pay")).not.toBeInTheDocument();
    expect(screen.queryByText("FRA 3×6")).not.toBeInTheDocument();
    // No rates risk columns.
    expect(screen.queryByRole("columnheader", { name: /^PV01$/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("columnheader", { name: /^DV01$/ })).not.toBeInTheDocument();
  });
});
