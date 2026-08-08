/**
 * URL hygiene — the address bar stays CLEAN during normal use (clean-bar change).
 *
 * The workspace / domain / model view state is NO LONGER mirrored into the query
 * string as the user navigates: clicking around the app (rail, tab-merges, command
 * palette, the acceptance alias) is purely in-memory (`setWorkspace`), so no
 * `view`/`dom`/`model` params accrete in the bar.
 *
 * Deep-links are still honoured on INITIAL LOAD ONLY: a pasted/bookmarked
 * `?view=…&dom=…&model=…` link seeds the first paint (see `domainDeepLink.test.tsx`)
 * and is then STRIPPED from the URL on mount so the bar is clean immediately after
 * landing. The non-view transport params `?ws=` (local-dev WS override) and `?mock`
 * (tests/e2e) are NOT view params, so they are preserved untouched.
 *
 * Renders the REAL Shell inside the REAL AppProvider (offline `?mock`) plus a small
 * `Probe` that reads the live workspace and drives an in-memory `setWorkspace`.
 */
import { act } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { AppProvider, useApp } from "../src/app/AppContext";
import { Shell } from "../src/app/Shell";
import { createMockTransport } from "../src/data/mockSource";

/** Reads the live workspace + drives an in-memory navigation (no URL touch). */
function Probe(): React.ReactElement {
  const app = useApp();
  return (
    <>
      <span data-testid="ws">{app.workspace}</span>
      <button type="button" data-testid="nav-risk" onClick={() => app.setWorkspace("risk")}>
        go-risk
      </button>
    </>
  );
}

async function renderAt(search: string): Promise<void> {
  window.history.replaceState(null, "", search);
  await act(async () => {
    render(
      <AppProvider>
        <Shell />
        <Probe />
      </AppProvider>,
    );
  });
  // Flush the mount effects (auth gating, the one-shot URL strip).
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

afterEach(() => {
  window.history.replaceState(null, "", "/");
  document.body.innerHTML = "";
});

function params(): URLSearchParams {
  return new URLSearchParams(window.location.search);
}

const VIEW_PARAMS = ["view", "dom", "scope", "group", "model", "meas", "axes", "trend"] as const;

function expectNoViewParams(): void {
  const p = params();
  for (const key of VIEW_PARAMS) {
    expect(p.has(key)).toBe(false);
  }
}

describe("URL hygiene — the address bar stays clean", () => {
  it("in-app navigation between workspaces adds NO view/dom/model params (mock preserved)", async () => {
    await renderAt("/?mock");
    // A bare (transport-only) URL is untouched by the mount strip.
    expect(window.location.search).toBe("?mock");

    await act(async () => {
      fireEvent.click(screen.getByTestId("nav-risk"));
      await Promise.resolve();
    });

    // The nav happened purely in memory…
    expect(screen.getByTestId("ws").textContent).toBe("risk");
    // …and the bar gained NO view-state params; `?mock` still stands.
    expectNoViewParams();
    expect(params().has("mock")).toBe(true);
  });

  it("a deep-link seeds the surface on load, then the view params are STRIPPED (mock preserved)", async () => {
    await renderAt("/?mock&view=acceptance&dom=fixed_income&model=MARKET_HEDGE");

    // The legacy link still routed us to its surface…
    expect(screen.getByTestId("ws").textContent).toBe("acceptance");
    // …but the bar is clean immediately after landing.
    expectNoViewParams();
    expect(params().has("mock")).toBe(true);
  });

  it("preserves ?ws= (local-dev override) and ?mock while stripping the view params", async () => {
    await renderAt("/?ws=wss%3A%2F%2Fexample%2F&mock&view=risk&dom=fixed_income");

    const p = params();
    expect(p.get("ws")).toBe("wss://example/");
    expect(p.has("mock")).toBe(true);
    expectNoViewParams();
  });

  it("leaves a bare URL untouched (nothing to strip)", async () => {
    // A truly bare URL has no `?mock`, so thread a mock transport in directly to
    // keep the provider offline (no live WS dial) while asserting the strip is a
    // no-op on an already-clean bar.
    window.history.replaceState(null, "", "/");
    await act(async () => {
      render(
        <AppProvider transport={createMockTransport()}>
          <Shell />
          <Probe />
        </AppProvider>,
      );
    });
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(window.location.search).toBe("");
  });
});
