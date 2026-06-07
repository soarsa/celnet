/**
 * StatusRibbon server-observability UX — renders the REAL `ServerObservabilityItems`
 * (the ribbon's observability cluster, a pure component over the distilled
 * `ServerObservability`) with hand-built values, and asserts the trader-facing
 * rendering: the drain-side price p99, the exact ring conflation-drop count, and
 * the surface/correlation provenance echo. Empty-state shows "—" (never a
 * fabricated zero); a non-zero drop count flags the warn class.
 */
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { ServerObservabilityItems } from "../src/app/StatusRibbon";
import type { ServerObservability } from "../src/hooks/useStreamSession";

const EMPTY: ServerObservability = {
  received: false,
  conflationDrops: 0n,
  serverPriceP99Nanos: 0n,
};

describe("StatusRibbon — server observability cluster", () => {
  it("shows the honest empty-state (— / — drops) before any beat", () => {
    render(<ServerObservabilityItems observability={EMPTY} />);
    expect(screen.getByTestId("server-p99").textContent).toContain("—");
    expect(screen.getByTestId("conflation-drops").textContent).toContain("— drops");
    // No provenance row until a beat carries an echo.
    expect(screen.queryByTestId("provenance-echo")).toBeNull();
  });

  it("renders the server price p99 in a scale-adaptive unit", () => {
    render(
      <ServerObservabilityItems
        observability={{ received: true, conflationDrops: 0n, serverPriceP99Nanos: 3100n }}
      />,
    );
    // 3100ns → 3.1µs (the format helper's sub-ms µs band).
    expect(screen.getByTestId("server-p99").textContent).toContain("3.1µs");
  });

  it("renders the exact conflation-drop count and flags a non-zero count", () => {
    render(
      <ServerObservabilityItems
        observability={{ received: true, conflationDrops: 16n, serverPriceP99Nanos: 4800n }}
      />,
    );
    const drops = screen.getByTestId("conflation-drops");
    expect(drops.textContent).toContain("16 drops");
    // A non-zero drop count must be visually warned, not silently green.
    expect(drops.className).toMatch(/warn/);
  });

  it("treats zero drops on a real beat as clean (no warn)", () => {
    render(
      <ServerObservabilityItems
        observability={{ received: true, conflationDrops: 0n, serverPriceP99Nanos: 900n }}
      />,
    );
    const drops = screen.getByTestId("conflation-drops");
    expect(drops.textContent).toContain("0 drops");
    expect(drops.className).not.toMatch(/warn/);
  });

  it("echoes the surface version and correlation id when present", () => {
    render(
      <ServerObservabilityItems
        observability={{
          received: true,
          conflationDrops: 0n,
          serverPriceP99Nanos: 900n,
          surfaceVersion: 9n,
          correlationId: 42n,
        }}
      />,
    );
    const echo = screen.getByTestId("provenance-echo");
    expect(echo.textContent).toContain("sv 9");
    expect(echo.textContent).toContain("corr 42");
  });
});
