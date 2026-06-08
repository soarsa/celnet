/**
 * Shell `?` → shortcuts cheatsheet end-to-end coverage (PC-SHORTCUTS).
 *
 * Renders the REAL `Shell` inside the REAL `AppProvider` (forced to the offline
 * in-app mock via `?mock`, so no server is dialled) and exercises the discovery
 * paths a user has:
 *   • pressing `?` opens the overlay (and toggling closes it);
 *   • `?` is ignored while typing into a text field (it's a literal char there);
 *   • the rail "?" button opens it;
 *   • the command palette carries a "Keyboard shortcuts" action.
 * These prove the binding the cheatsheet advertises (`?`) is the one the Shell
 * actually honours — the no-drift contract.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { Shell } from "../src/app/Shell";

beforeEach(() => {
  window.history.replaceState(null, "", "/?mock");
});
afterEach(() => {
  window.history.replaceState(null, "", "/");
  document.body.innerHTML = "";
});

async function renderShell(): Promise<void> {
  await act(async () => {
    render(
      <AppProvider>
        <Shell />
      </AppProvider>,
    );
  });
  // Let the provider's async mount settle.
  await act(async () => {
    await Promise.resolve();
  });
}

function dialog(): HTMLElement | null {
  return screen.queryByRole("dialog", { name: "Keyboard shortcuts" });
}

describe("Shell — `?` keyboard cheatsheet discovery", () => {
  it("pressing `?` opens the cheatsheet and pressing it again closes it", async () => {
    await renderShell();
    expect(dialog()).toBeNull();

    act(() => {
      fireEvent.keyDown(window, { key: "?" });
    });
    expect(dialog()).not.toBeNull();

    act(() => {
      fireEvent.keyDown(window, { key: "?" });
    });
    expect(dialog()).toBeNull();
  });

  it("ignores `?` while typing into a text input (literal character)", async () => {
    await renderShell();
    // Open the command palette, which focuses a real <input>. A `?` typed INTO that
    // field is a literal character, so it must NOT open the cheatsheet (the Shell
    // guards on the event target being an INPUT/TEXTAREA).
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: /Search \/ command/i }));
    });
    const field = document.querySelector("input");
    expect(field).not.toBeNull();
    act(() => {
      fireEvent.keyDown(field as HTMLElement, { key: "?" });
    });
    expect(dialog()).toBeNull();
  });

  it("the rail `?` button opens the cheatsheet", async () => {
    await renderShell();
    fireEvent.click(screen.getByRole("button", { name: "show keyboard shortcuts" }));
    expect(dialog()).not.toBeNull();
  });

  it("the command palette offers a cheatsheet action that opens it", async () => {
    await renderShell();
    // ⌘K opens the palette.
    act(() => {
      fireEvent.keyDown(window, { key: "k", metaKey: true });
    });
    // The palette fuzzy-filters to a top-N result list; type to surface the action.
    // The action is the registry's "help" command, labelled "Show the keyboard
    // cheatsheet" (GW1: the palette renders the single command registry).
    const palette = await screen.findByRole("dialog", { name: "command palette" });
    const field = palette.querySelector("input");
    expect(field).not.toBeNull();
    act(() => {
      fireEvent.change(field as HTMLInputElement, { target: { value: "cheatsheet" } });
    });
    const action = await screen.findByText("Show the keyboard cheatsheet");
    act(() => {
      fireEvent.mouseDown(action);
    });
    expect(dialog()).not.toBeNull();
  });
});
