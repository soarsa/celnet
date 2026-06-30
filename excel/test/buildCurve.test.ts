/**
 * BuildCurve codec parity (Excel add-in). The Excel client speaks the SAME
 * type-tagged snake_case wire as the GUI and the server's `ws/codec.rs`: the
 * `build_curve` request body and the `calibrated_curve` response frame must encode
 * / decode field-for-field identically across every client (CLAUDE.md #9 — one
 * clean current contract). This asserts the round-trip on the Excel codec.
 */
import { describe, expect, it } from "vitest";

import type {
  BuildCurveRequest,
  CalibratedCurve,
} from "../src/contract/contract";
import {
  buildCurveRequestToWire,
  calibratedCurveFromWire,
} from "../src/contract/wsCodec";
import type { WireObject } from "../src/contract/wsCodec";

const REQUEST: BuildCurveRequest = {
  requestId: "curve-001",
  currency: "USD",
  referenceDate: { year: 2026, month: 6, day: 25 },
  pillars: [
    { instrumentId: "usd-sofr-depo-3m", quote: 0.0431 },
    { instrumentId: "usd-sofr-irs-10y", quote: 0.0418 },
  ],
};

describe("Excel wsCodec — BuildCurve", () => {
  it("encodes the snake_case build_curve body (connection injects auth)", () => {
    const wire = buildCurveRequestToWire(REQUEST);
    expect(wire).toEqual({
      request_id: "curve-001",
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      pillars: [
        { instrument_id: "usd-sofr-depo-3m", quote: 0.0431 },
        { instrument_id: "usd-sofr-irs-10y", quote: 0.0418 },
      ],
    });
    expect("session_token" in wire).toBe(false);
  });

  it("decodes a server-shaped calibrated_curve frame losslessly", () => {
    const frame: WireObject = {
      request_id: "curve-001",
      currency: "USD",
      reference_date: { year: 2026, month: 6, day: 25 },
      points: [
        {
          instrument_id: "usd-sofr-depo-3m",
          time_years: 0.2521,
          discount_factor: 0.98912,
          zero_rate: 0.04318,
        },
        {
          instrument_id: "usd-sofr-irs-10y",
          time_years: 10.0,
          discount_factor: 0.6612,
          zero_rate: 0.04134,
        },
      ],
    };
    const curve: CalibratedCurve = calibratedCurveFromWire(frame);
    expect(curve).toEqual({
      requestId: "curve-001",
      currency: "USD",
      referenceDate: { year: 2026, month: 6, day: 25 },
      points: [
        {
          instrumentId: "usd-sofr-depo-3m",
          timeYears: 0.2521,
          discountFactor: 0.98912,
          zeroRate: 0.04318,
        },
        {
          instrumentId: "usd-sofr-irs-10y",
          timeYears: 10.0,
          discountFactor: 0.6612,
          zeroRate: 0.04134,
        },
      ],
    });
  });

  it("round-trips the request header through the wire (server echoes it back)", () => {
    const wire = buildCurveRequestToWire(REQUEST);
    const echoed: WireObject = {
      request_id: wire.request_id,
      currency: wire.currency,
      reference_date: wire.reference_date,
      points: [],
    };
    const curve = calibratedCurveFromWire(echoed);
    expect(curve.requestId).toBe(REQUEST.requestId);
    expect(curve.currency).toBe(REQUEST.currency);
    expect(curve.referenceDate).toEqual(REQUEST.referenceDate);
  });
});
