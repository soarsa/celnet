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
  it("renders every registered spec, grouped under its labelled ProductGroup", () => {
    render(<StructureGallery value={PRODUCT_REGISTRY[0]!.id} onSelect={() => {}} />);

    const options = screen.getAllByRole("option");
    expect(options).toHaveLength(PRODUCT_REGISTRY.length);
    for (const spec of PRODUCT_REGISTRY) {
      expect(screen.getByText(spec.label)).toBeInTheDocument();
    }

    // One group section per non-empty ProductGroup, in canonical order. The
    // group labels are PRESENTATIONAL (a listbox may only own group/option
    // children — axe aria-required-children), so they are read off each group's
    // aria-labelledby target, NOT a heading role.
    const expectedGroups = registryByGroup().map((g) => g.group);
    const labels = screen.getAllByRole("group").map((grp) => {
      const labelEl = document.getElementById(grp.getAttribute("aria-labelledby")!);
      expect(labelEl).toHaveAttribute("role", "presentation");
      return labelEl?.textContent;
    });
    expect(labels).toEqual(expectedGroups);
    // Every rendered group label is a real ProductGroup.
    for (const l of labels) expect(PRODUCT_GROUP_ORDER).toContain(l);
    // The corrected ARIA contract: NO heading role may leak inside the listbox.
    expect(screen.queryAllByRole("heading")).toHaveLength(0);
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

/**
 * Asset-class-aware discovery (multi-asset wave): the gallery surfaces the
 * capability matrix as guidance — FX/metal keep the full catalogue; on a true
 * cross-asset class the FX/metal-only families are dimmed + non-selectable with the
 * honest reason, and a now-unpriceable selection is auto-reselected.
 */
describe("StructureGallery — asset-class awareness", () => {
  const REASON = /only vanilla, perpetual and listed-future-option/i;

  it("FX (and the default) keeps the full catalogue with no dimmed cards", () => {
    render(<StructureGallery value="VANILLA" assetClass="FX" onSelect={() => {}} />);
    expect(screen.getAllByRole("option")).toHaveLength(PRODUCT_REGISTRY.length);
    expect(document.querySelectorAll('[aria-disabled="true"]')).toHaveLength(0);
    expect(screen.queryAllByText(REASON)).toHaveLength(0);
  });

  it("EQUITY offers the cross-asset vanilla, dims FX-only families, and blocks selecting them", () => {
    const onSelect = vi.fn();
    render(<StructureGallery value="CROSS_ASSET_VANILLA" assetClass="EQUITY" onSelect={onSelect} />);

    // The cross-asset vanilla is the available builder (a real, selectable option).
    expect(screen.getByText("Cross-asset vanilla").closest('[role="option"]')).not.toBeNull();
    // FX/metal-only families render dimmed (aria-disabled) with the honest reason.
    const disabled = document.querySelectorAll('[aria-disabled="true"]');
    expect(disabled.length).toBeGreaterThan(0);
    expect(screen.queryAllByText(REASON).length).toBeGreaterThan(0);
    // value is available ⇒ no auto-reselect on mount; a dimmed click does nothing.
    expect(onSelect).not.toHaveBeenCalled();
    fireEvent.click(disabled[0] as Element);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("auto-reselects an available family when the selection becomes unpriceable on the class", () => {
    const onSelect = vi.fn();
    render(<StructureGallery value="SINGLE_BARRIER" assetClass="EQUITY" onSelect={onSelect} />);
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect.mock.calls[0]?.[0]).not.toBe("SINGLE_BARRIER");
  });
});
