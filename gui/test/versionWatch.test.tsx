/**
 * useVersionWatch (the deploy-detection polling shell) + UpdateBanner (the auto-
 * reloading notice) under jsdom. The pure comparison / reload-orchestration logic is
 * covered in versionManifest.test.ts; here we exercise the React seam: the no-store
 * poll latches a newer build, an equal build is ignored, and the banner fires its
 * reload EXACTLY ONCE (countdown or manual), never looping.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import type { CelnetTransport } from "../src/data/transport";
import {
  resetReleaseLatch,
  useVersionWatch,
  type ReleaseManifest,
} from "../src/data/versionManifest";
import { UpdateBanner } from "../src/app/UpdateBanner";

const running: ReleaseManifest = { hash: "v0.0.0", buildTime: "2026-06-27T13:00:00.000Z" };
const newer: ReleaseManifest = { hash: "v0.0.0", buildTime: "2026-06-27T14:00:00.000Z" };

/** A transport with no live socket — the `?mock` shape (no `onConnectionState`). */
const mockTransport = {} as unknown as CelnetTransport;

/** Stub `fetch` to return `served` from `/version.json` with an assertable call log. */
function stubVersionFetch(served: ReleaseManifest) {
  const fetchMock = vi.fn(async () => ({
    ok: true,
    json: async () => served,
  }));
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

function Harness({ init }: { init?: ReleaseManifest }): React.ReactElement {
  const { available } = useVersionWatch(mockTransport, init ?? running);
  return <div data-testid="avail">{available ? available.buildTime : "none"}</div>;
}

beforeEach(() => {
  resetReleaseLatch();
});

afterEach(() => {
  cleanup();
  resetReleaseLatch();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("useVersionWatch", () => {
  it("polls /version.json with cache:no-store and latches a strictly newer build", async () => {
    const fetchMock = stubVersionFetch(newer);
    render(<Harness />);

    await waitFor(() =>
      expect(screen.getByTestId("avail")).toHaveTextContent(newer.buildTime),
    );
    expect(fetchMock).toHaveBeenCalledWith("/version.json", { cache: "no-store" });
  });

  it("does NOT latch when the served build equals the running build", async () => {
    const fetchMock = stubVersionFetch({ ...running });
    render(<Harness />);

    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByTestId("avail")).toHaveTextContent("none");
  });
});

describe("UpdateBanner", () => {
  it("announces the update as a live status region", () => {
    render(<UpdateBanner release={newer} onReload={vi.fn()} countdownSeconds={5} />);
    const region = screen.getByRole("status");
    expect(region).toHaveTextContent("Updating to the latest version");
    expect(region).toHaveAttribute("aria-live", "polite");
    expect(screen.getByRole("button", { name: "Reload now" })).toBeInTheDocument();
  });

  it("auto-reloads EXACTLY ONCE after the countdown, then never again", () => {
    vi.useFakeTimers();
    const onReload = vi.fn();
    render(<UpdateBanner release={newer} onReload={onReload} countdownSeconds={3} />);

    expect(onReload).not.toHaveBeenCalled();
    act(() => void vi.advanceTimersByTime(3000));
    expect(onReload).toHaveBeenCalledTimes(1);
    // Keep advancing — the fired-latch must not let it repeat (no reload loop).
    act(() => void vi.advanceTimersByTime(10_000));
    expect(onReload).toHaveBeenCalledTimes(1);
  });

  it("'Reload now' fires immediately and suppresses the later countdown fire", () => {
    vi.useFakeTimers();
    const onReload = vi.fn();
    render(<UpdateBanner release={newer} onReload={onReload} countdownSeconds={5} />);

    fireEvent.click(screen.getByRole("button", { name: "Reload now" }));
    expect(onReload).toHaveBeenCalledTimes(1);
    act(() => void vi.advanceTimersByTime(6000));
    expect(onReload).toHaveBeenCalledTimes(1);
  });
});
