/**
 * SettingsPanel — the per-event notifications config UI. These tests drive the
 * real panel wrapped in the real SettingsProvider (settings persist to
 * localStorage), with only the sound-kit `previewSound` spied so no audio runs.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";

const { previewSpy } = vi.hoisted(() => ({ previewSpy: vi.fn() }));
vi.mock("../src/lib/soundKit", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/lib/soundKit")>();
  return { ...actual, previewSound: previewSpy };
});

import { SettingsPanel } from "../src/components/SettingsPanel";
import { SettingsProvider } from "../src/settings/SettingsProvider";
import {
  SETTINGS_STORAGE_KEY,
  DEFAULT_PER_EVENT,
} from "../src/settings/settingsSchema";

function open() {
  render(
    <SettingsProvider>
      <SettingsPanel />
    </SettingsProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
}

function storedPerEvent() {
  const raw = window.localStorage.getItem(SETTINGS_STORAGE_KEY);
  return JSON.parse(raw ?? "{}").perEvent;
}

describe("SettingsPanel — per-event notifications", () => {
  beforeEach(() => {
    window.localStorage.clear();
    previewSpy.mockClear();
  });
  afterEach(() => cleanup());

  it("renders one configurable row per event type", () => {
    open();
    const labels = [
      "RFQ received",
      "IOI received",
      "Needs manual pricing",
      "Quote accepted (won)",
      "Quote rejected (lost)",
      "Withdrawn / expired",
      "Order received",
      "Fill",
      "Block fill",
    ];
    for (const label of labels) {
      expect(screen.getByRole("group", { name: label })).toBeTruthy();
    }
  });

  it("the ▶ preview button plays the row's configured cue", () => {
    open();
    fireEvent.click(screen.getByRole("button", { name: "Preview RFQ received sound" }));
    expect(previewSpy).toHaveBeenCalledTimes(1);
    // masterVolume 70 × RfqReceived trim 100% ⇒ 70; cue = the configured rfq-work.
    expect(previewSpy).toHaveBeenCalledWith("rfq-work", 70);
  });

  it("toggling a channel checkbox persists the per-event change", () => {
    open();
    const row = screen.getByRole("group", { name: "RFQ received" });
    // RfqReceived defaults to toast-only; enabling Desktop should persist.
    expect(DEFAULT_PER_EVENT.RfqReceived.channels.desktop).toBe(false);
    const desktop = within(row).getByRole("checkbox", { name: "Desktop" });
    fireEvent.click(desktop);
    expect(storedPerEvent().RfqReceived.channels.desktop).toBe(true);
  });

  it("changing a row's sound persists the new cue", () => {
    open();
    const row = screen.getByRole("group", { name: "Quote accepted (won)" });
    const select = within(row).getByLabelText("Quote accepted (won) sound");
    fireEvent.change(select, { target: { value: "celebrate" } });
    expect(storedPerEvent().QuoteAccepted.sound).toBe("celebrate");
  });

  it("disabling an event persists enabled=false", () => {
    open();
    const row = screen.getByRole("group", { name: "Fill" });
    const toggle = within(row).getByRole("switch", { name: "Fill enabled" });
    fireEvent.click(toggle);
    expect(storedPerEvent().Fill.enabled).toBe(false);
  });
});
