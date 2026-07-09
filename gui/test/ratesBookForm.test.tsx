import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { AppProvider } from "../src/app/AppContext";
import { RatesBookWorkspace } from "../src/workspaces/RatesBookWorkspace";
import { entityNameOf } from "../src/workspaces/RegistryPanels";

/**
 * The rates Book ticket must let a trader pick a NAMED legal entity and a NAMED
 * book (resolving the selection to the `RatesPosition`'s `(entity, book)` uint32
 * keys — the position wire is unchanged) and must render entity/book NAMES, never
 * raw numbers, in the positions table. Driven against the offline `MockTransport`
 * (seeded to mirror the server registry) through the real `AppProvider`.
 */

beforeEach(() => {
  // The rates book is fixed-income (OIS): its booking ticket + positions only
  // render under the Fixed Income domain (hard asset separation). Seed the FI
  // domain via the `dom` deep-link so the ticket's entity/book selects mount.
  window.history.replaceState(null, "", "/?mock&dom=fixed_income");
});
afterEach(() => {
  window.history.replaceState(null, "", "/");
  document.body.innerHTML = "";
});

async function renderWorkspace(): Promise<void> {
  await act(async () => {
    render(
      <AppProvider>
        <RatesBookWorkspace />
      </AppProvider>,
    );
  });
  await act(async () => {
    await Promise.resolve();
  });
}

describe("rates Book ticket — named entity / book dropdowns", () => {
  it("renders Entity and Book as named <select> dropdowns from the registry", async () => {
    await renderWorkspace();

    const entitySelect = (await screen.findByLabelText("legal entity")) as HTMLSelectElement;
    expect(entitySelect.tagName).toBe("SELECT");
    const entityOptions = Array.from(entitySelect.options).map((o) => o.textContent);
    expect(entityOptions).toContain("Celnet Global Markets");
    expect(entityOptions).toContain("Celnet Securities");
    // The keys are carried as the option VALUES (the wire uint32 partition keys),
    // never shown as the label.
    expect(Array.from(entitySelect.options).map((o) => o.value)).toContain("1");

    const bookSelect = screen.getByLabelText("netting book") as HTMLSelectElement;
    expect(bookSelect.tagName).toBe("SELECT");
  });

  it("filters the Book dropdown to the selected entity's books", async () => {
    await renderWorkspace();
    const entitySelect = (await screen.findByLabelText("legal entity")) as HTMLSelectElement;
    const bookSelect = screen.getByLabelText("netting book") as HTMLSelectElement;

    // Default entity (Celnet Global Markets, key 1) → its books only.
    let bookLabels = Array.from(bookSelect.options).map((o) => o.textContent);
    expect(bookLabels).toEqual(["Rates Trading", "Rates Relative Value"]);

    // Re-target to Celnet Securities (key 2) → its books only.
    await act(async () => {
      fireEvent.change(entitySelect, { target: { value: "2" } });
    });
    bookLabels = Array.from(bookSelect.options).map((o) => o.textContent);
    expect(bookLabels).toEqual(["Government Bonds", "Swaps"]);
  });

  it("resolves the named selection to numeric keys on submit and shows NAMES in the book", async () => {
    await renderWorkspace();
    const entitySelect = (await screen.findByLabelText("legal entity")) as HTMLSelectElement;
    const bookSelect = screen.getByLabelText("netting book") as HTMLSelectElement;

    // Pick Celnet Securities (key 2) / Government Bonds (key 3).
    await act(async () => {
      fireEvent.change(entitySelect, { target: { value: "2" } });
    });
    await act(async () => {
      fireEvent.change(bookSelect, { target: { value: "3" } });
    });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Book position" }));
    });

    // The booked line appears in the positions table showing the resolved NAMES
    // (the selection resolved to keys 2/3 on the wire, displayed as names here).
    const table = await screen.findByRole("table");
    await waitFor(() => {
      expect(within(table).getAllByText("Celnet Securities").length).toBeGreaterThan(0);
      expect(within(table).getAllByText("Government Bonds").length).toBeGreaterThan(0);
    });
    // No row shows a raw entity/book number: every body row's entity + book cell
    // is a resolved name, so the numeric selection keys (2/3) never surface as the
    // entity/book column text (the position id column is the only numeric column).
    const rows = within(table).getAllByRole("row").slice(1); // drop the header row
    for (const row of rows) {
      const cells = within(row).getAllByRole("cell");
      expect(cells[1]?.textContent).not.toMatch(/^\d+$/); // entity column = a name
      expect(cells[2]?.textContent).not.toMatch(/^\d+$/); // book column = a name
    }
  });
});

describe("registry name resolution", () => {
  it("resolves a known key to its name and an unknown key to #<key>", () => {
    const entities = [
      { key: 1, name: "Celnet Global Markets", code: "CGM" },
      { key: 2, name: "Celnet Securities", code: "CSEC" },
    ];
    expect(entityNameOf(entities, 1)).toBe("Celnet Global Markets");
    expect(entityNameOf(entities, 99)).toBe("#99");
  });
});
