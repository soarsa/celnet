/**
 * Component gate for the {@link Panel} primitive's keyboard-scroll contract
 * (WCAG 2.1.1 / axe `scrollable-region-focusable`): the `overflow: auto` body
 * must be keyboard-reachable EXACTLY when its content genuinely overflows —
 * `tabIndex=0` plus a `region` role named by the panel title — and must add no
 * tab stop (and no role) when it does not. jsdom has no layout engine, so the
 * overflow geometry is driven by temporarily shadowing the metric getters.
 */
import { afterEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { Panel } from "../src/components/Panel";

/** Shadow scroll/client metrics on HTMLElement (jsdom reports 0 for all). */
function mockOverflow(scrollHeight: number, clientHeight: number): void {
  Object.defineProperty(HTMLElement.prototype, "scrollHeight", {
    configurable: true,
    get: () => scrollHeight,
  });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get: () => clientHeight,
  });
}

function restoreMetrics(): void {
  // Deleting the shadow getters re-exposes jsdom's own prototype metrics.
  delete (HTMLElement.prototype as { scrollHeight?: unknown }).scrollHeight;
  delete (HTMLElement.prototype as { clientHeight?: unknown }).clientHeight;
}

afterEach(restoreMetrics);

describe("Panel scrollable-body keyboard access", () => {
  it("makes an overflowing body a focusable region named by the panel title", () => {
    mockOverflow(400, 120);
    render(
      <Panel title="Vega ladder" glyph="Σ">
        <div>rows</div>
      </Panel>,
    );

    const body = screen.getByRole("region", { name: "Vega ladder" });
    expect(body).toHaveAttribute("tabindex", "0");
    expect(body).toContainElement(screen.getByText("rows"));
  });

  it("keeps an overflowing TITLE-LESS body focusable without a nameless region role", () => {
    mockOverflow(400, 120);
    const { container } = render(
      <Panel>
        <div>content</div>
      </Panel>,
    );

    const body = screen.getByText("content").parentElement!;
    expect(body).toHaveAttribute("tabindex", "0");
    // A region landmark without an accessible name is worse than none.
    expect(container.querySelector('[role="region"]')).toBeNull();
  });

  it("adds NO tab stop and NO region role when the body does not overflow", () => {
    // jsdom default: every metric is 0 — nothing overflows.
    const { container } = render(
      <Panel title="Marking">
        <div>fits</div>
      </Panel>,
    );

    expect(container.querySelector("[tabindex]")).toBeNull();
    expect(container.querySelector('[role="region"]')).toBeNull();
  });
});
