import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  getFdc3Agent,
  instrumentToFdc3,
  fdc3ToInstrument,
  type Fdc3InstrumentContext,
  type Fdc3OrderContext,
  type Fdc3TradeContext,
  type Fdc3ValuationContext,
} from "../src/lib/fdc3";

describe("FDC3 v2.1 Compliance & Interoperability", () => {
  beforeEach(() => {
    // Reset global window.fdc3 if present
    if (typeof window !== "undefined") {
      delete (window as unknown as { fdc3?: unknown }).fdc3;
    }
  });

  it("converts FX tickers to standard FDC3 Instrument context", () => {
    const ctx = instrumentToFdc3("EUR/USD");
    expect(ctx.type).toBe("fdc3.instrument");
    expect(ctx.name).toBe("EUR/USD Spot Exchange Rate");
    expect(ctx.id.ticker).toBe("EUR/USD");
    expect(ctx.id.RIC).toBe("EURUSD=");
    expect(ctx.market?.MIC).toBe("XOFF");
  });

  it("converts Equity tickers to standard FDC3 Instrument context", () => {
    const ctx = instrumentToFdc3("AAPL");
    expect(ctx.type).toBe("fdc3.instrument");
    expect(ctx.name).toBe("AAPL");
    expect(ctx.id.ticker).toBe("AAPL");
    expect(ctx.id.RIC).toBe("AAPL.O");
    expect(ctx.market?.MIC).toBe("XNAS");
  });

  it("parses tickers correctly from inbound FDC3 contexts", () => {
    const fxCtx: Fdc3InstrumentContext = {
      type: "fdc3.instrument",
      id: { ticker: "GBP/USD" },
    };
    expect(fdc3ToInstrument(fxCtx)).toBe("GBP/USD");

    const ricCtx: Fdc3InstrumentContext = {
      type: "fdc3.instrument",
      id: { RIC: "USDJPY=" },
    };
    expect(fdc3ToInstrument(ricCtx)).toBe("USDJPY");

    const invalidCtx = { type: "fdc3.unknown", id: {} };
    expect(fdc3ToInstrument(invalidCtx as any)).toBeNull();
  });

  it("initializes Desktop Agent and manages channel switching", async () => {
    const agent = getFdc3Agent();
    expect(agent).toBeDefined();

    await agent.joinUserChannel("red");
    const current = await agent.getCurrentChannel();
    expect(current?.id).toBe("fdc3.channel.1");
    expect(current?.displayMetadata?.name).toBe("Red");

    await agent.joinUserChannel("blue");
    const blueChannel = await agent.getCurrentChannel();
    expect(blueChannel?.id).toBe("fdc3.channel.5");
  });

  it("supports context broadcasting and listener subscription", async () => {
    const agent = getFdc3Agent();
    await agent.joinUserChannel("green");

    const received: any[] = [];
    const listener = agent.addContextListener("fdc3.instrument", (ctx) => {
      received.push(ctx);
    });

    const context = instrumentToFdc3("EUR/USD");
    await agent.broadcast(context);

    expect(received.length).toBe(1);
    expect(received[0].id.ticker).toBe("EUR/USD");

    listener.unsubscribe();

    // After unsubscribe, no new calls
    await agent.broadcast(instrumentToFdc3("USD/JPY"));
    expect(received.length).toBe(1);
  });

  it("supports FDC3 intent raising and handling", async () => {
    const agent = getFdc3Agent();
    const intentCalls: any[] = [];

    const intentListener = agent.addIntentListener("ViewChart", (ctx) => {
      intentCalls.push(ctx);
    });

    const ctx = instrumentToFdc3("NVDA");
    const result = await agent.raiseIntent("ViewChart", ctx);

    expect((result as any).status).toBe("ACK");
    expect(intentCalls.length).toBe(1);
    expect(intentCalls[0].id.ticker).toBe("NVDA");

    intentListener.unsubscribe();
  });

  it("validates FDC3 Valuation Context structure", () => {
    const valCtx: Fdc3ValuationContext = {
      type: "fdc3.valuation",
      instrument: instrumentToFdc3("EUR/USD"),
      metrics: {
        pv: 12450.50,
        delta: 0.542,
        gamma: 0.012,
        vega: 450.20,
        theta: -34.10,
        currency: "USD",
      },
    };

    expect(valCtx.type).toBe("fdc3.valuation");
    expect(valCtx.metrics.pv).toBe(12450.50);
    expect(valCtx.metrics.delta).toBe(0.542);
    expect(valCtx.metrics.currency).toBe("USD");
  });

  it("validates FDC3 Order Context structure", () => {
    const orderCtx: Fdc3OrderContext = {
      type: "fdc3.order",
      id: { orderId: "ORD-998822" },
      details: {
        type: "LIMIT",
        side: "BUY",
        quantity: 1000000,
        price: 1.0850,
        timeInForce: "IOC",
      },
    };

    expect(orderCtx.type).toBe("fdc3.order");
    expect(orderCtx.details?.side).toBe("BUY");
    expect(orderCtx.details?.price).toBe(1.0850);
  });
});
