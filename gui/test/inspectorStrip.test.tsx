/**
 * InspectorStrip shell (GW0 lane A): the per-Panel analytics-config header.
 *
 * The strip is a composable container: a lane declares only the analytics
 * segments that apply to it (model|measures|axes|trend|columns|density|saved-view),
 * the strip renders them in ONE canonical order regardless of declaration order,
 * and the trailing <Provenance> slot is always present. GW1+ fills the segment
 * content; here we prove the grammar — ordering, selective population, the always-
 * present provenance slot, and the accessible toolbar role.
 */

import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";

import { InspectorStrip, INSPECTOR_SEGMENTS } from "../src/components/InspectorStrip";
import { Provenance } from "../src/components/Provenance";

describe("<InspectorStrip>", () => {
  it("is an accessible analytics toolbar named for its panel", () => {
    render(<InspectorStrip label="Surface" />);
    const strip = screen.getByRole("toolbar", { name: "Surface analytics" });
    expect(strip).toBeInTheDocument();
  });

  it("renders only the segments a lane declares (selective population)", () => {
    render(
      <InspectorStrip
        label="Ticket"
        segments={[
          { id: "model", label: "model", content: <span>chips</span> },
          { id: "trend", label: "trend", content: <span>1m</span> },
        ]}
      />,
    );
    const strip = screen.getByRole("toolbar");
    expect(strip.querySelector('[data-segment="model"]')).not.toBeNull();
    expect(strip.querySelector('[data-segment="trend"]')).not.toBeNull();
    // Segments the lane did NOT declare are absent (no empty axes/columns chrome).
    expect(strip.querySelector('[data-segment="axes"]')).toBeNull();
    expect(strip.querySelector('[data-segment="columns"]')).toBeNull();
    expect(within(strip).getByText("chips")).toBeInTheDocument();
  });

  it("renders segments in the canonical order regardless of declaration order", () => {
    render(
      <InspectorStrip
        label="Cube"
        segments={[
          { id: "saved-view", content: <span>sv</span> },
          { id: "model", content: <span>m</span> },
          { id: "axes", content: <span>ax</span> },
          { id: "density", content: <span>d</span> },
        ]}
      />,
    );
    const strip = screen.getByRole("toolbar");
    const order = Array.from(strip.querySelectorAll("[data-segment]")).map((el) =>
      el.getAttribute("data-segment"),
    );
    // Filter to the analytics segments (the trailing slot is "provenance").
    const segmentsOnly = order.filter((id) => id !== "provenance");
    // Canonical order: model before axes before density before saved-view.
    expect(segmentsOnly).toEqual(["model", "axes", "density", "saved-view"]);
    // Provenance always trails as the final slot.
    expect(order[order.length - 1]).toBe("provenance");
  });

  it("always provides a provenance slot, even with no provenance node", () => {
    render(<InspectorStrip label="Risk" />);
    const strip = screen.getByRole("toolbar");
    const slot = strip.querySelector('[data-segment="provenance"]');
    expect(slot).not.toBeNull();
    expect(slot!.textContent).toBe("");
  });

  it("docks a <Provenance> into the trailing slot", () => {
    render(
      <InspectorStrip
        label="Surface"
        segments={[{ id: "model", content: <span>m</span> }]}
        provenance={<Provenance source="desk surface" model="extended-surface" />}
      />,
    );
    const strip = screen.getByRole("toolbar");
    const slot = strip.querySelector('[data-segment="provenance"]') as HTMLElement;
    expect(within(slot).getByLabelText(/provenance/)).toBeInTheDocument();
    expect(slot.textContent).toContain("desk surface");
  });

  it("exposes the canonical segment vocabulary for lanes to declare against", () => {
    expect(INSPECTOR_SEGMENTS).toEqual([
      "model",
      "measures",
      "axes",
      "trend",
      "columns",
      "density",
      "saved-view",
    ]);
  });
});
