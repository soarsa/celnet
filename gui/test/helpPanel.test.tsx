/**
 * HelpPanel render + interaction: the rich panel surfaces every section of a
 * registry entry (purpose, how-it-works, the worked example, config steps, risks),
 * closes on Escape / the close button, and launches the entry's tour via
 * "Walk me through it".
 */
import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, test, vi } from "vitest";

import { HelpPanel } from "../src/components/HelpPanel";
import { getHelp } from "../src/lib/help";

const tiering = getHelp("feature.tiering")!;

describe("HelpPanel", () => {
  test("renders the topic's title, purpose and every content section", () => {
    render(<HelpPanel entry={tiering} onClose={() => {}} />);
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByRole("heading", { name: "TIERING" })).toBeInTheDocument();
    expect(within(dialog).getByText(tiering.purpose)).toBeInTheDocument();
    expect(within(dialog).getByText("How it works")).toBeInTheDocument();
    expect(within(dialog).getByText("Worked example")).toBeInTheDocument();
    expect(within(dialog).getByText("How to configure")).toBeInTheDocument();
    expect(within(dialog).getByText("Risks")).toBeInTheDocument();
  });

  test("renders the worked example's real numbers (99.30 / 99.80)", () => {
    render(<HelpPanel entry={tiering} onClose={() => {}} />);
    expect(screen.getAllByText(/99\.30/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/99\.80/).length).toBeGreaterThan(0);
  });

  test("renders every how-to-configure step as a list item", () => {
    render(<HelpPanel entry={tiering} onClose={() => {}} />);
    const items = screen.getAllByRole("listitem");
    expect(items.length).toBeGreaterThanOrEqual(tiering.howToConfigure.length);
  });

  test("the close button calls onClose", () => {
    const onClose = vi.fn();
    render(<HelpPanel entry={tiering} onClose={onClose} />);
    fireEvent.click(screen.getByRole("button", { name: /Close help/ }));
    expect(onClose).toHaveBeenCalledOnce();
  });

  test("Escape closes the panel", () => {
    const onClose = vi.fn();
    render(<HelpPanel entry={tiering} onClose={onClose} />);
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(onClose).toHaveBeenCalledOnce();
  });

  test("“Walk me through it” launches the tour and closes the panel", () => {
    const onClose = vi.fn();
    const onStartTour = vi.fn();
    render(<HelpPanel entry={tiering} onClose={onClose} onStartTour={onStartTour} />);
    fireEvent.click(screen.getByRole("button", { name: /Walk me through it/ }));
    expect(onStartTour).toHaveBeenCalledWith(tiering.tourId);
    expect(onClose).toHaveBeenCalledOnce();
  });

  test("without an onStartTour handler there is no tour button", () => {
    render(<HelpPanel entry={tiering} onClose={() => {}} />);
    expect(screen.queryByRole("button", { name: /Walk me through it/ })).not.toBeInTheDocument();
  });
});
