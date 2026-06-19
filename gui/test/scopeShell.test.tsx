/**
 * GW1 shell + scope integration (GW1-S1/S2/S3).
 *
 * Renders the REAL Shell inside the REAL AppProvider (offline `?mock`) and proves:
 *   • the data-driven rail renders N views with the derived ⌘N hints, Book carries
 *     the ledger glyph ▤ (the Σ→▤ fix), and Σ is NOT a rail glyph;
 *   • the ONE scope control drills DOWN (via the switcher) and back UP, and the
 *     group-by control serialises into the scope state;
 *   • the four redundant pair affordances are GONE — there is exactly one scope
 *     control and the dead PairMenu/PairStrip modules are not imported anywhere
 *     (the no-legacy guard).
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { Shell } from "../src/app/Shell";
import { RAIL, railChord } from "../src/lib/commands";

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
  await act(async () => {
    await Promise.resolve();
  });
}

describe("data-driven rail + glyph fix (GW1-S1)", () => {
  it("renders one rail button per registry view with its ⌘N hint (admin-only views hidden)", async () => {
    await renderShell();
    const rail = screen.getByRole("complementary", { name: "workspaces" });
    // The default render is a non-admin (anonymous) session: every registry view
    // appears EXCEPT the admin-only Connections workspace, and each visible button
    // keeps its registry ⌘N hint (the original index, so numbering stays aligned).
    for (let i = 0; i < RAIL.length; i += 1) {
      const r = RAIL[i]!;
      if (r.id === "connections") {
        expect(
          within(rail).queryByRole("button", { name: new RegExp(r.label, "i") }),
        ).toBeNull();
        continue;
      }
      const btn = within(rail).getByRole("button", { name: new RegExp(r.label, "i") });
      expect(btn.getAttribute("title")).toContain(railChord(i).join(""));
    }
  });

  it("Book uses the ledger glyph ▤ and Σ is not a rail glyph (one glyph, one meaning)", async () => {
    await renderShell();
    const rail = screen.getByRole("complementary", { name: "workspaces" });
    const book = RAIL.find((r) => r.id === "book")!;
    expect(book.glyph).toBe("▤");
    // No rail glyph is Σ — it is freed for sum / vega-ladder use exclusively.
    expect(RAIL.some((r) => r.glyph === "Σ")).toBe(false);
    expect(within(rail).getByRole("button", { name: /Book/i }).textContent).toContain("▤");
  });
});

describe("the ONE scope control: drill up / down / pin (GW1-S2/S3)", () => {
  it("drills DOWN via the switcher and back UP via an ancestor crumb", async () => {
    await renderShell();
    const scope = screen.getByRole("navigation", { name: "scope" });
    // At the firm root the implied-span descriptor is shown and only Firm crumb.
    expect(within(scope).getByText("Firm")).toBeInTheDocument();

    // Drill DOWN one level: the drill button opens the scope switcher (the leaf).
    fireEvent.click(screen.getByRole("button", { name: /drill into a desk/i }));
    const switcher = await screen.findByRole("dialog", { name: "Desks" });
    // Pick the first desk → a desk crumb appears in the breadcrumb.
    const firstDesk = within(switcher).getAllByRole("option")[0]!;
    const deskLabel = firstDesk.textContent ?? "";
    act(() => {
      fireEvent.click(firstDesk);
    });
    const scope2 = screen.getByRole("navigation", { name: "scope" });
    expect(within(scope2).getByText(deskLabel.trim())).toBeInTheDocument();

    // Drill back UP to the Firm ancestor crumb (truncates the path). The crumb's
    // accessible name is its text "Firm" (the title is supplementary).
    fireEvent.click(within(scope2).getByRole("button", { name: "Firm" }));
    const scope3 = screen.getByRole("navigation", { name: "scope" });
    expect(within(scope3).queryByText(deskLabel.trim())).toBeNull();
    expect(within(scope3).getByText("Firm")).toBeInTheDocument();
  });

  it("the group-by control pins the secondary axis into the scope state", async () => {
    await renderShell();
    const groupBy = screen.getByRole("combobox", { name: "group by" }) as HTMLSelectElement;
    expect(groupBy.value).toBe("none");
    act(() => {
      fireEvent.change(groupBy, { target: { value: "desk" } });
    });
    expect((screen.getByRole("combobox", { name: "group by" }) as HTMLSelectElement).value).toBe(
      "desk",
    );
  });
});

describe("no-legacy: the four redundant pair affordances are absorbed (GW1-S3)", () => {
  it("renders exactly one scope control and no separate 'Pairs' button / strip", async () => {
    await renderShell();
    // The single scope nav exists.
    expect(screen.getAllByRole("navigation", { name: "scope" })).toHaveLength(1);
    // The deleted "Pairs" browse button is gone.
    expect(screen.queryByRole("button", { name: /browse the pair universe/i })).toBeNull();
    // The deleted PairStrip watchlist nav is gone.
    expect(screen.queryByRole("navigation", { name: /currency pair watchlist/i })).toBeNull();
  });

  it("the dead pair-affordance modules are deleted and imported nowhere", () => {
    const srcRoot = join(__dirname, "../src");
    function* walk(dir: string): Generator<string> {
      for (const entry of readdirSync(dir)) {
        const full = join(dir, entry);
        if (statSync(full).isDirectory()) yield* walk(full);
        else if (/\.(ts|tsx)$/.test(entry)) yield full;
      }
    }
    const dead = ["PairMenu", "PairStrip", "ScopeBreadcrumb", "UniverseNavigator"];
    const offenders: string[] = [];
    for (const file of walk(srcRoot)) {
      const text = readFileSync(file, "utf8");
      for (const name of dead) {
        // The real dangling-reference signal is an IMPORT of the deleted module or
        // a JSX render of the deleted component — NOT a prose mention in a doc
        // comment explaining the migration (those are allowed; they document the
        // absorbed affordances). Match an import path `.../Name` or a `<Name` tag.
        const importRe = new RegExp(`from\\s+["'][^"']*/${name}["']`);
        const jsxRe = new RegExp(`<${name}[\\s/>]`);
        if (importRe.test(text) || jsxRe.test(text)) {
          offenders.push(`${file.replace(srcRoot, "src")} → ${name}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});
