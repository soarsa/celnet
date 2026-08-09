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
import {
  ADMIN_ONLY_WORKSPACES,
  RAIL,
  railChord,
  railForDomain,
} from "../src/lib/commands";

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

describe("data-driven single class-parametric rail + glyph fix (GW1-S1 / #6)", () => {
  it("renders the ACTIVE (default FX) domain's rail rows with their ⌘N hints (others hidden)", async () => {
    await renderShell();
    const rail = screen.getByRole("complementary", { name: "workspaces" });
    // fe-fi-migration re-add: the rail is FILTERED to the active domain (default
    // fx_options). Every FX-domain row appears with its registry ⌘N hint (the hint
    // still derives from the FULL RAIL index — ⌘1..⌘9, ⌘0 for the tenth); an FI-only
    // row (Quoting) and the anonymous-hidden admin/ops panes are NOT in the FX rail.
    const fxRows = new Set(railForDomain("fx_options").map((r) => r.id));
    const escapeRe = (s: string): string => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    for (let i = 0; i < RAIL.length; i += 1) {
      const r = RAIL[i]!;
      // Disambiguate by the UNIQUE subtitle when present: the consolidated FI "Risk"
      // host and the FX "Risk" scenario row share the label "Risk", so a label-only
      // regex would ambiguously match. A subtitled row's aria-label is `label —
      // subtitle`; a row without a subtitle has none, so its accessible name is
      // `glyph label` — matched on the label alone.
      const matcher = r.subtitle
        ? new RegExp(escapeRe(r.subtitle))
        : new RegExp(`\\b${escapeRe(r.label)}\\b`);
      const btn = within(rail).queryByRole("button", { name: matcher });
      const shouldShow = fxRows.has(r.id) && !ADMIN_ONLY_WORKSPACES.has(r.id);
      if (!shouldShow) {
        expect(btn).toBeNull();
        continue;
      }
      expect(btn).not.toBeNull();
      const chord = railChord(i).join("");
      if (chord.length > 0) expect(btn!.getAttribute("title")).toContain(chord);
    }
    // The FI-only Quoting row is NOT reachable from the FX tab.
    expect(within(rail).queryByRole("button", { name: /Quoting/i })).toBeNull();
  });

  it("Book uses the ledger glyph ▤ and Σ is not a rail glyph (one glyph, one meaning)", async () => {
    await renderShell();
    const book = RAIL.find((r) => r.id === "book")!;
    expect(book.glyph).toBe("▤");
    // No rail glyph is Σ — it is freed for sum / vega-ladder use exclusively.
    expect(RAIL.some((r) => r.glyph === "Σ")).toBe(false);
    // The single rail shows Book directly — no domain tab to select first, and the
    // "Rates Book" twin was collapsed away, so the Book button is unambiguous.
    const rail = screen.getByRole("complementary", { name: "workspaces" });
    const bookBtn = within(rail)
      .getAllByRole("button")
      .find((b) => b.getAttribute("title")?.startsWith("Book "));
    expect(bookBtn?.textContent).toContain("▤");
  });
});

describe("product-domain tab bar (fe-fi-migration re-add — Model A)", () => {
  it("shows FX Options + Fixed Income tabs signed out; Administration hidden; FX active", async () => {
    await renderShell();
    const tablist = screen.getByRole("tablist", { name: "product domains" });
    const names = within(tablist)
      .getAllByRole("tab")
      .map((t) => t.textContent ?? "");
    expect(names.some((n) => n.includes("FX Options"))).toBe(true);
    expect(names.some((n) => n.includes("Fixed Income"))).toBe(true);
    // Administration is isAdmin-gated ⇒ hidden for the anonymous session.
    expect(names.some((n) => n.includes("Administration"))).toBe(false);
    expect(
      within(tablist).getByRole("tab", { name: /FX Options/i }).getAttribute("aria-selected"),
    ).toBe("true");
  });

  it("selecting Fixed Income filters the rail (Quoting appears, Stream hidden) + selects the FI tab", async () => {
    await renderShell();
    const tablist = screen.getByRole("tablist", { name: "product domains" });
    let rail = screen.getByRole("complementary", { name: "workspaces" });
    // Under the FX tab: Stream (FX-only) present, Quoting + Streaming (FI-only)
    // absent. `/Stream$/i` matches the FX "Stream" row but NOT the FI "Streaming"
    // row (which ends in "…ing"), so the two rows are disambiguated cleanly.
    expect(within(rail).queryByRole("button", { name: /Stream$/i })).not.toBeNull();
    expect(within(rail).queryByRole("button", { name: /Streaming/i })).toBeNull();
    expect(within(rail).queryByRole("button", { name: /Quoting/i })).toBeNull();
    act(() => {
      fireEvent.click(within(tablist).getByRole("tab", { name: /Fixed Income/i }));
    });
    rail = screen.getByRole("complementary", { name: "workspaces" });
    // Under the FI tab: Quoting + Streaming (FI) present, the FX-only Stream hidden
    // — the rail follows the domain.
    expect(within(rail).queryByRole("button", { name: /Quoting/i })).not.toBeNull();
    expect(within(rail).queryByRole("button", { name: /Streaming/i })).not.toBeNull();
    expect(within(rail).queryByRole("button", { name: /Stream$/i })).toBeNull();
    expect(
      within(tablist).getByRole("tab", { name: /Fixed Income/i }).getAttribute("aria-selected"),
    ).toBe("true");
  });

  it("switching tabs on a SHARED screen keeps the screen and renders the new domain's single asset (hard separation — no lens toggle)", async () => {
    await renderShell();
    // Enter the shared Market Data screen from the FX tab.
    act(() => {
      fireEvent.click(
        within(screen.getByRole("complementary", { name: "workspaces" })).getByRole("button", {
          name: /Market Data/i,
        }),
      );
    });
    // The shared Market-Data row is context-relabelled per domain — "Market Data"
    // under FX, "Curves" under Fixed Income — so match either.
    const mdBtn = (): HTMLElement =>
      within(screen.getByRole("complementary", { name: "workspaces" })).getByRole("button", {
        name: /Market Data|Curves/i,
      });
    // Hard vertical asset separation: on the FX tab the shared Market Data screen
    // renders ONLY the FX (vol-surface) lens — its own "surface view" sub-control
    // is present — and there is NO cross-asset lens toggle.
    expect(mdBtn().getAttribute("aria-current")).toBe("true");
    expect(await screen.findByRole("group", { name: "surface view" })).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "market data asset class" })).toBeNull();
    // The FI (curve) lens is not rendered under the FX domain — no FI tab/content.
    expect(screen.queryByRole("tablist", { name: "curves manager lens" })).toBeNull();

    // Flip to the Fixed Income tab: the SHARED Market Data screen is KEPT…
    act(() => {
      fireEvent.click(
        within(screen.getByRole("tablist", { name: "product domains" })).getByRole("tab", {
          name: /Fixed Income/i,
        }),
      );
    });
    expect(mdBtn().getAttribute("aria-current")).toBe("true"); // still on Market Data
    // …and it now renders ONLY the FI (rates curve) lens, derived from the domain —
    // still no cross-asset toggle, and the FX surface lens is gone.
    expect(
      await screen.findByRole("tablist", { name: "curves manager lens" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "market data asset class" })).toBeNull();
    expect(screen.queryByRole("group", { name: "surface view" })).toBeNull();
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
