/**
 * Component gate for the {@link StructureGallery} (GW2) — the grouped, searchable,
 * keyboard-first structure picker that replaces the flat ticket `<select>`. We
 * drive it through the REAL registry (no mocked specs) and assert the
 * accessibility contract (listbox / option / group, aria-selected,
 * aria-activedescendant), selection by click + keyboard, and search narrowing
 * with an honest empty state.
 */
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { StructureGallery } from "../../src/products/StructureGallery";
import { PRODUCT_REGISTRY, PRODUCT_GROUP_ORDER, registryByGroup } from "../../src/products/index";

describe("StructureGallery", () => {
  it("renders every registered spec, grouped under its ProductGroup heading", () => {
    render(<StructureGallery value={PRODUCT_REGISTRY[0]!.id} onSelect={() => {}} />);

    const options = screen.getAllByRole("option");
    expect(options).toHaveLength(PRODUCT_REGISTRY.length);
    for (const spec of PRODUCT_REGISTRY) {
      expect(screen.getByText(spec.label)).toBeInTheDocument();
    }

    // One group section per non-empty ProductGroup, in canonical order.
    const expectedGroups = registryByGroup().map((g) => g.group);
    const headings = screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent);
    expect(headings).toEqual(expectedGroups);
    // Every rendered heading is a real ProductGroup.
    for (const h of headings) expect(PRODUCT_GROUP_ORDER).toContain(h);
  });

  it("marks exactly the selected card aria-selected=true", () => {
    const target = PRODUCT_REGISTRY[3]!;
    render(<StructureGallery value={target.id} onSelect={() => {}} />);

    const selected = screen
      .getAllByRole("option")
      .filter((el) => el.getAttribute("aria-selected") === "true");
    expect(selected).toHaveLength(1);
    expect(within(selected[0]!).getByText(target.label)).toBeInTheDocument();
  });

  it("calls onSelect with the spec id when a card is clicked", () => {
    const onSelect = vi.fn();
    const other = PRODUCT_REGISTRY[5]!;
    render(<StructureGallery value={PRODUCT_REGISTRY[0]!.id} onSelect={onSelect} />);

    fireEvent.click(screen.getByText(other.label).closest('[role="option"]')!);
    expect(onSelect).toHaveBeenCalledWith(other.id);
  });

  it("narrows the list as the search query is typed (label/summary/keywords)", () => {
    render(<StructureGallery value={PRODUCT_REGISTRY[0]!.id} onSelect={() => {}} />);
    const search = screen.getByRole("searchbox", { name: /search structures/i });

    fireEvent.change(search, { target: { value: "asian" } });
    const after = screen.getAllByRole("option");
    expect(after.length).toBeLessThan(PRODUCT_REGISTRY.length);
    expect(after.length).toBeGreaterThan(0);
    expect(screen.getByText("Asian (average-rate)")).toBeInTheDocument();
  });

  it("shows an honest empty state when nothing matches", () => {
    render(<StructureGallery value={PRODUCT_REGISTRY[0]!.id} onSelect={() => {}} />);
    fireEvent.change(screen.getByRole("searchbox", { name: /search structures/i }), {
      target: { value: "zzz-no-such-structure" },
    });

    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(screen.getByText(/no structures match/i)).toBeInTheDocument();
  });

  it("moves the active descendant and selection with ArrowDown + selects with Enter", () => {
    const onSelect = vi.fn();
    const first = PRODUCT_REGISTRY[0]!;
    render(<StructureGallery value={first.id} onSelect={onSelect} />);

    const listbox = screen.getByRole("listbox");
    // aria-activedescendant starts on the selected (first) card.
    const optionId = listbox.getAttribute("aria-activedescendant");
    expect(optionId).toBeTruthy();
    expect(document.getElementById(optionId!)).toHaveAttribute("aria-selected", "true");

    // ArrowDown advances selection to the next visible card.
    fireEvent.keyDown(listbox, { key: "ArrowDown" });
    expect(onSelect).toHaveBeenLastCalledWith(PRODUCT_REGISTRY[1]!.id);

    // Enter re-asserts the active card (the selected one in this controlled test).
    fireEvent.keyDown(listbox, { key: "Enter" });
    expect(onSelect).toHaveBeenLastCalledWith(first.id);
  });

  it("respects an injected specs subset (scoped catalogue)", () => {
    const subset = PRODUCT_REGISTRY.slice(0, 2);
    render(
      <StructureGallery value={subset[0]!.id} onSelect={() => {}} specs={subset} />,
    );
    expect(screen.getAllByRole("option")).toHaveLength(2);
  });

  it("exposes the listbox/option/group a11y contract with no orphan roles", () => {
    render(<StructureGallery value={PRODUCT_REGISTRY[0]!.id} onSelect={() => {}} />);

    const listbox = screen.getByRole("listbox");
    expect(listbox).toHaveAttribute("aria-labelledby");
    expect(listbox).toHaveAttribute("tabindex", "0");

    // Every option carries an explicit aria-selected boolean and a stable id.
    for (const opt of screen.getAllByRole("option")) {
      expect(opt).toHaveAttribute("aria-selected");
      expect(opt.id).toBeTruthy();
    }
    // Every group is labelled by a heading that exists in the document.
    for (const grp of screen.getAllByRole("group")) {
      const labelledBy = grp.getAttribute("aria-labelledby");
      expect(labelledBy).toBeTruthy();
      expect(document.getElementById(labelledBy!)).toBeInTheDocument();
    }
  });
});
