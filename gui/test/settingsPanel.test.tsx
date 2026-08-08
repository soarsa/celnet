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

describe("SettingsPanel — centered popup modal", () => {
  beforeEach(() => {
    window.localStorage.clear();
    previewSpy.mockClear();
  });
  afterEach(() => cleanup());

  it("the gear opens a centered role=dialog modal (closed by default)", () => {
    render(
      <SettingsProvider>
        <SettingsPanel />
      </SettingsProvider>,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const dialog = screen.getByRole("dialog", { name: "Settings" });
    expect(dialog).toBeTruthy();
    // Labelled centered modal on a scrim (rendered via a portal to <body>).
    const scrim = dialog.parentElement as HTMLElement;
    expect(scrim.getAttribute("role")).toBe("presentation");
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    // Settings controls still render inside the modal.
    expect(within(dialog).getByRole("switch", { name: "In-app alerts" })).toBeTruthy();
  });

  it("moves focus into the dialog on open and restores it to the gear on Escape", () => {
    render(
      <SettingsProvider>
        <SettingsPanel />
      </SettingsProvider>,
    );
    const gear = screen.getByRole("button", { name: "Settings" });
    fireEvent.click(gear);
    const dialog = screen.getByRole("dialog", { name: "Settings" });
    expect(document.activeElement).toBe(dialog);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(gear);
  });

  it("the X button closes the modal and restores focus to the gear", () => {
    render(
      <SettingsProvider>
        <SettingsPanel />
      </SettingsProvider>,
    );
    const gear = screen.getByRole("button", { name: "Settings" });
    fireEvent.click(gear);
    fireEvent.click(screen.getByRole("button", { name: "Close settings" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(gear);
  });

  it("a scrim (outside) click closes the modal", () => {
    render(
      <SettingsProvider>
        <SettingsPanel />
      </SettingsProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const dialog = screen.getByRole("dialog", { name: "Settings" });
    const scrim = dialog.parentElement as HTMLElement;
    // A mousedown whose target IS the scrim (not the panel) dismisses.
    fireEvent.mouseDown(scrim);
    expect(screen.queryByRole("dialog")).toBeNull();
    // A mousedown on the panel does NOT close.
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const dialog2 = screen.getByRole("dialog", { name: "Settings" });
    fireEvent.mouseDown(dialog2);
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeTruthy();
  });

  it("shows a Version row sourced from the running build identity", () => {
    render(
      <SettingsProvider>
        <SettingsPanel />
      </SettingsProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const foot = screen.getByTestId("settings-version");
    expect(within(foot).getByText("Version")).toBeTruthy();
    // vitest defines the running identity as hash "test" (a placeholder) + this
    // build time — the placeholder branch surfaces the human build time instead.
    expect(within(foot).getByText("1970-01-01 00:00 UTC")).toBeTruthy();
  });
});

describe("SettingsPanel — Version row with a real build hash", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.resetModules();
  });
  afterEach(() => cleanup());

  it("renders the short hash + human build time when the hash is a real git identity", async () => {
    vi.doMock("../src/data/versionManifest", async (importOriginal) => {
      const actual = await importOriginal<typeof import("../src/data/versionManifest")>();
      return {
        ...actual,
        RUNNING_RELEASE: { hash: "9f3c1ab", buildTime: "2026-06-27T13:25:28.000Z" },
      };
    });
    // Re-import BOTH from the fresh module graph so the panel and provider share
    // the same SettingsContext instance (vi.resetModules forked the registry).
    const { SettingsPanel: Panel } = await import("../src/components/SettingsPanel");
    const { SettingsProvider: Provider } = await import("../src/settings/SettingsProvider");
    render(
      <Provider>
        <Panel />
      </Provider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const foot = screen.getByTestId("settings-version");
    expect(within(foot).getByText("9f3c1ab")).toBeTruthy();
    expect(within(foot).getByText("2026-06-27 13:25 UTC")).toBeTruthy();
    vi.doUnmock("../src/data/versionManifest");
  });
});
