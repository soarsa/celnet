/**
 * Honest-data primitives (GW0 lane A): <EmptyValue>, <StdError>, <Provenance>.
 *
 * These components encode the platform's data-honesty as composable building
 * blocks. The properties under test are the honesty contract itself:
 *  - <EmptyValue> renders the em-dash "—" with an accessible REASON, never `0`
 *    and never a blank — the absence is explicit and machine-describable.
 *  - <StdError> mirrors `Quote.price_std_error` (field 7): closed-form
 *    (`undefined` / `0`) renders NOTHING (never a fabricated "± 0"), and a
 *    Monte-Carlo error (`> 0`) renders a "± x" precision band. The honesty
 *    property is asserted over the full `{undefined, 0, >0}` domain so a future
 *    edit that treats `0` as present would fail here.
 *  - <Provenance> names the source/model/as-of of a mark and omits absent
 *    segments (metadata, not a data cell); with nothing to say it renders null.
 */

import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { EmptyValue } from "../src/components/EmptyValue";
import { StdError } from "../src/components/StdError";
import { Provenance } from "../src/components/Provenance";

describe("<EmptyValue>", () => {
  it("renders an em-dash, never 0 or blank", () => {
    render(<EmptyValue />);
    const el = screen.getByRole("img");
    expect(el.textContent).toBe("—");
    expect(el.textContent).not.toBe("0");
    expect(el.textContent).not.toBe("");
  });

  it("carries an accessible default reason", () => {
    render(<EmptyValue />);
    const el = screen.getByRole("img", { name: "no value" });
    expect(el).toHaveAttribute("title", "no value");
  });

  it("surfaces a caller-supplied reason (the honest WHY)", () => {
    render(<EmptyValue reason="no calibration for this tenor" />);
    const el = screen.getByRole("img", { name: "no calibration for this tenor" });
    expect(el).toHaveAttribute("title", "no calibration for this tenor");
    expect(el.textContent).toBe("—");
  });
});

describe("<StdError> — the MC precision honesty contract", () => {
  // The honest-data property over the full domain. Closed-form families report no
  // error; MC families report a positive standard error. The boundary `0` is
  // treated as closed-form-grade (no band), never "± 0".
  it.each([
    { value: undefined as number | undefined, present: false, label: "closed form (undefined)" },
    { value: 0, present: false, label: "MC reported zero error (0)" },
    { value: 0.0012, present: true, label: "MC error (>0)" },
  ])("$label → band present: $present", ({ value, present }) => {
    const { container } = render(<StdError value={value} />);
    const node = screen.queryByLabelText("price std error");
    if (present) {
      expect(node).not.toBeNull();
      // Renders a positive "± x" band as a percentage of premium (value × 100,
      // 4dp): 0.0012 → ±0.1200. The zero case is the `present:false` row.
      expect(node!.textContent).toContain("±0.1200");
    } else {
      expect(node).toBeNull();
      // Renders literally nothing (no fabricated band, no stray chrome).
      expect(container.textContent).toBe("");
    }
  });

  it("expresses the band as a percentage of PV when a non-zero PV is supplied", () => {
    render(<StdError value={0.01} pv={0.5} />);
    const node = screen.getByLabelText("price std error");
    // 0.01 / 0.5 = 2.00% of PV.
    expect(node.textContent).toContain("2.00% of PV");
  });

  it("omits the % when PV is zero (no divide-by-zero illusion)", () => {
    render(<StdError value={0.01} pv={0} />);
    const node = screen.getByLabelText("price std error");
    expect(node.textContent).not.toContain("% of PV");
  });

  it("drops the Monte-Carlo provenance label when unlabelled", () => {
    render(<StdError value={0.01} labelled={false} />);
    const node = screen.getByLabelText("price std error");
    expect(node.textContent).not.toContain("Monte-Carlo");
    expect(node.textContent).toContain("±");
  });
});

describe("<Provenance>", () => {
  it("names source, model and as-of when present", () => {
    render(<Provenance source="desk surface" model="extended-surface" asOf="sv 42" />);
    const el = screen.getByLabelText(/provenance/);
    expect(el.textContent).toContain("desk surface");
    expect(el.textContent).toContain("extended-surface");
    expect(el.textContent).toContain("sv 42");
    expect(el).toHaveAttribute(
      "title",
      "source: desk surface · model: extended-surface · as of: sv 42",
    );
  });

  it("omits absent segments rather than rendering empty placeholders", () => {
    render(<Provenance source="vendor" />);
    const el = screen.getByLabelText(/provenance/);
    expect(el.textContent).toContain("vendor");
    expect(el.textContent).not.toContain("model");
    expect(el).toHaveAttribute("title", "source: vendor");
  });

  it("renders nothing when it has nothing to attribute", () => {
    const { container } = render(<Provenance />);
    expect(container.textContent).toBe("");
  });

  it("treats an empty-string segment as absent", () => {
    render(<Provenance source="" model="parametric" />);
    const el = screen.getByLabelText(/provenance/);
    expect(el.textContent).toContain("parametric");
    expect(el.textContent).not.toContain("source");
  });
});
