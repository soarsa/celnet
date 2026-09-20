/**
 * CelNet FDC3 v2.1 Interoperability & Desktop Connectivity Engine
 *
 * Implements the Financial Desktop Connectivity and Collaboration Standard (FDC3 v2.1)
 * established by FINOS (Fintech Open Source Foundation).
 *
 * Features:
 * - Standard Context Types: fdc3.instrument, fdc3.position, fdc3.order, fdc3.trade, fdc3.valuation
 * - Standard Intents: ViewChart, ViewOrders, ViewQuote, Trade, ViewAnalysis, ViewInstrument
 * - Desktop Container Integration: OpenFin, interop.io (Glue42), Finsemble
 * - Modern Browser Fallback: BroadcastChannel('fdc3-interop-v2.1') for native multi-window tearing
 * - Channel Management: System color channels (red, green, blue, orange, purple, yellow, global)
 */

export interface Fdc3InstrumentContext {
  type: "fdc3.instrument";
  name?: string;
  id: {
    ticker?: string;
    ISIN?: string;
    FIGI?: string;
    RIC?: string;
    SEDOL?: string;
    [key: string]: string | undefined;
  };
  market?: {
    MIC?: string;
    name?: string;
  };
}

export interface Fdc3PositionContext {
  type: "fdc3.position";
  instrument: Fdc3InstrumentContext;
  holding: number;
  currency?: string;
  valuation?: {
    value: number;
    currency: string;
    timestamp?: string;
  };
}

export interface Fdc3OrderContext {
  type: "fdc3.order";
  id: {
    orderId: string;
    [key: string]: string | undefined;
  };
  details?: {
    type?: "MARKET" | "LIMIT" | "STOP";
    side?: "BUY" | "SELL";
    quantity?: number;
    price?: number;
    timeInForce?: "GTC" | "IOC" | "FOK" | "DAY";
  };
}

export interface Fdc3TradeContext {
  type: "fdc3.trade";
  id: {
    tradeId: string;
    uti?: string;
    [key: string]: string | undefined;
  };
  details?: {
    tradeTime?: string;
    counterparty?: string;
    price?: number;
    quantity?: number;
    status?: "NEW" | "ALLOCATED" | "CONFIRMED" | "CLEARED" | "SETTLED" | "TERMINATED";
  };
}

export interface Fdc3ValuationContext {
  type: "fdc3.valuation";
  instrument: Fdc3InstrumentContext;
  metrics: {
    pv: number;
    delta?: number;
    gamma?: number;
    vega?: number;
    theta?: number;
    rho?: number;
    currency: string;
  };
}

export type Fdc3Context =
  | Fdc3InstrumentContext
  | Fdc3PositionContext
  | Fdc3OrderContext
  | Fdc3TradeContext
  | Fdc3ValuationContext
  | { type: string; id?: Record<string, string | undefined>; [key: string]: unknown };

export type Fdc3ChannelColor =
  | "global"
  | "red"
  | "orange"
  | "yellow"
  | "green"
  | "blue"
  | "magenta"
  | "purple"
  | "cyan";

export interface Fdc3ChannelMetadata {
  id: string;
  name: string;
  color: string;
  colorVar: string;
  bgVar: string;
}

export const FDC3_USER_CHANNELS: readonly Fdc3ChannelMetadata[] = [
  { id: "global", name: "Global", color: "#94a3b8", colorVar: "--text-muted", bgVar: "rgba(148,163,184,0.15)" },
  { id: "fdc3.channel.1", name: "Red", color: "#f87171", colorVar: "--brand-coral", bgVar: "rgba(248,113,113,0.15)" },
  { id: "fdc3.channel.2", name: "Orange", color: "#fb923c", colorVar: "--brand-amber", bgVar: "rgba(251,146,60,0.15)" },
  { id: "fdc3.channel.3", name: "Yellow", color: "#facc15", colorVar: "--brand-yellow", bgVar: "rgba(250,204,21,0.15)" },
  { id: "fdc3.channel.4", name: "Green", color: "#4ade80", colorVar: "--brand-mint", bgVar: "rgba(74,222,128,0.15)" },
  { id: "fdc3.channel.5", name: "Blue", color: "#38bdf8", colorVar: "--brand-blue", bgVar: "rgba(56,189,248,0.15)" },
  { id: "fdc3.channel.6", name: "Magenta", color: "#e879f9", colorVar: "--brand-magenta", bgVar: "rgba(232,121,249,0.15)" },
  { id: "fdc3.channel.7", name: "Purple", color: "#c084fc", colorVar: "--brand-purple", bgVar: "rgba(192,132,252,0.15)" },
  { id: "fdc3.channel.8", name: "Cyan", color: "#2dd4bf", colorVar: "--brand-cyan", bgVar: "rgba(45,212,191,0.15)" },
];

export function normalizeChannelId(channel: string): string {
  const c = channel.toLowerCase().trim();
  if (c === "red" || c === "1") return "fdc3.channel.1";
  if (c === "orange" || c === "2") return "fdc3.channel.2";
  if (c === "yellow" || c === "3") return "fdc3.channel.3";
  if (c === "green" || c === "4") return "fdc3.channel.4";
  if (c === "blue" || c === "5") return "fdc3.channel.5";
  if (c === "magenta" || c === "6") return "fdc3.channel.6";
  if (c === "purple" || c === "7") return "fdc3.channel.7";
  if (c === "cyan" || c === "8") return "fdc3.channel.8";
  if (c === "global" || c === "default") return "global";
  return c;
}

export function channelIdToMetadata(channelId: string): Fdc3ChannelMetadata {
  const norm = normalizeChannelId(channelId);
  return (
    FDC3_USER_CHANNELS.find((c) => c.id === norm) || {
      id: norm,
      name: norm.toUpperCase(),
      color: "#94a3b8",
      colorVar: "--text-muted",
      bgVar: "rgba(148,163,184,0.15)",
    }
  );
}

export interface Fdc3Channel {
  id: string;
  type: "user" | "app" | "private";
  displayMetadata?: {
    name?: string;
    color?: string;
    glyph?: string;
  };
  broadcast(context: Fdc3Context): Promise<void>;
  getCurrentContext(contextType?: string): Promise<Fdc3Context | null>;
  addContextListener(
    contextType: string | null,
    handler: (context: Fdc3Context) => void
  ): { unsubscribe: () => void };
}

export interface Fdc3DesktopAgent {
  joinUserChannel(channelId: string): Promise<void>;
  getCurrentChannel(): Promise<Fdc3Channel | null>;
  getUserChannels(): Promise<Fdc3Channel[]>;
  broadcast(context: Fdc3Context): Promise<void>;
  addContextListener(
    contextType: string | null,
    handler: (context: Fdc3Context) => void
  ): { unsubscribe: () => void };
  addIntentListener(
    intent: string,
    handler: (context: Fdc3Context) => unknown
  ): { unsubscribe: () => void };
  raiseIntent(intent: string, context: Fdc3Context, target?: string): Promise<unknown>;
}

// In-browser BroadcastChannel Fallback for cross-window / cross-tab multi-monitor setups
class WebBroadcastDesktopAgent implements Fdc3DesktopAgent {
  private currentChannelId: string = "fdc3.channel.4"; // default to Green (channel 4)
  private broadcastChannel: BroadcastChannel | null = null;
  private listeners: Map<string, Set<(ctx: Fdc3Context) => void>> = new Map();
  private intentListeners: Map<string, Set<(ctx: Fdc3Context) => unknown>> = new Map();
  private channelCache: Map<string, Fdc3Context> = new Map();

  constructor() {
    if (typeof window !== "undefined" && "BroadcastChannel" in window) {
      try {
        this.broadcastChannel = new BroadcastChannel("celnet-fdc3-v2.1");
        this.broadcastChannel.onmessage = (event: MessageEvent) => {
          this.handleInboundMessage(event.data);
        };
      } catch (err) {
        console.warn("FDC3: BroadcastChannel unavailable, using in-memory bus", err);
      }
    }
  }

  private handleInboundMessage(data: {
    type: "broadcast" | "intent";
    channelId?: string;
    intent?: string;
    context: Fdc3Context;
  }): void {
    if (!data || !data.context) return;

    if (data.type === "broadcast") {
      if (data.channelId === this.currentChannelId || data.channelId === "global") {
        this.dispatchContext(data.context);
      }
    } else if (data.type === "intent" && data.intent) {
      const handlers = this.intentListeners.get(data.intent);
      if (handlers) {
        handlers.forEach((h) => {
          try {
            h(data.context);
          } catch (e) {
            console.error("FDC3 intent handler error:", e);
          }
        });
      }
    }
  }

  private dispatchContext(context: Fdc3Context): void {
    this.channelCache.set(`${this.currentChannelId}:${context.type}`, context);

    // Specific listeners
    const typed = this.listeners.get(context.type);
    if (typed) {
      typed.forEach((h) => h(context));
    }

    // Wildcard listeners
    const wildcard = this.listeners.get("*");
    if (wildcard) {
      wildcard.forEach((h) => h(context));
    }
  }

  async joinUserChannel(channelId: string): Promise<void> {
    const norm = normalizeChannelId(channelId);
    this.currentChannelId = norm;
    try {
      sessionStorage.setItem("celnet:fdc3:channel", norm);
    } catch {}
  }

  async getCurrentChannel(): Promise<Fdc3Channel | null> {
    const meta = channelIdToMetadata(this.currentChannelId);
    return {
      id: this.currentChannelId,
      type: "user",
      displayMetadata: {
        name: meta.name,
        color: meta.color,
      },
      broadcast: async (ctx: Fdc3Context) => this.broadcast(ctx),
      getCurrentContext: async (type?: string) => {
        if (!type) return null;
        return this.channelCache.get(`${this.currentChannelId}:${type}`) || null;
      },
      addContextListener: (type, handler) => this.addContextListener(type, handler),
    };
  }

  async getUserChannels(): Promise<Fdc3Channel[]> {
    return FDC3_USER_CHANNELS.map((meta) => ({
      id: meta.id,
      type: "user",
      displayMetadata: { name: meta.name, color: meta.color },
      broadcast: async (ctx) => {
        if (this.broadcastChannel) {
          this.broadcastChannel.postMessage({ type: "broadcast", channelId: meta.id, context: ctx });
        }
        if (this.currentChannelId === meta.id) {
          this.dispatchContext(ctx);
        }
      },
      getCurrentContext: async (t) => (t ? this.channelCache.get(`${meta.id}:${t}`) || null : null),
      addContextListener: (t, h) => this.addContextListener(t, h),
    }));
  }

  async broadcast(context: Fdc3Context): Promise<void> {
    if (this.broadcastChannel) {
      this.broadcastChannel.postMessage({
        type: "broadcast",
        channelId: this.currentChannelId,
        context,
      });
    }
    this.dispatchContext(context);
  }

  addContextListener(
    contextType: string | null,
    handler: (context: Fdc3Context) => void
  ): { unsubscribe: () => void } {
    const key = contextType || "*";
    if (!this.listeners.has(key)) {
      this.listeners.set(key, new Set());
    }
    this.listeners.get(key)!.add(handler);

    return {
      unsubscribe: () => {
        const set = this.listeners.get(key);
        if (set) set.delete(handler);
      },
    };
  }

  addIntentListener(
    intent: string,
    handler: (context: Fdc3Context) => unknown
  ): { unsubscribe: () => void } {
    if (!this.intentListeners.has(intent)) {
      this.intentListeners.set(intent, new Set());
    }
    this.intentListeners.get(intent)!.add(handler);

    return {
      unsubscribe: () => {
        const set = this.intentListeners.get(intent);
        if (set) set.delete(handler);
      },
    };
  }

  async raiseIntent(intent: string, context: Fdc3Context, _target?: string): Promise<unknown> {
    if (this.broadcastChannel) {
      this.broadcastChannel.postMessage({
        type: "intent",
        intent,
        context,
      });
    }
    const handlers = this.intentListeners.get(intent);
    if (handlers) {
      handlers.forEach((h) => h(context));
    }
    return { status: "ACK", intent };
  }
}

// Global Singleton Agent Resolution
let activeAgent: Fdc3DesktopAgent | null = null;

export function getFdc3Agent(): Fdc3DesktopAgent {
  if (activeAgent) return activeAgent;

  if (typeof window !== "undefined" && (window as unknown as { fdc3?: Fdc3DesktopAgent }).fdc3) {
    activeAgent = (window as unknown as { fdc3: Fdc3DesktopAgent }).fdc3;
    return activeAgent;
  }

  activeAgent = new WebBroadcastDesktopAgent();
  if (typeof window !== "undefined") {
    (window as unknown as { fdc3?: Fdc3DesktopAgent }).fdc3 = activeAgent;
  }
  return activeAgent;
}

/**
 * Convert CelNet Instrument string (e.g. "EUR/USD", "XAU/USD", "AAPL") to FDC3 Instrument Context.
 */
export function instrumentToFdc3(ticker: string): Fdc3InstrumentContext {
  const clean = ticker.trim().toUpperCase();
  const isFx = clean.includes("/") || (clean.length === 6 && !clean.includes(" "));
  
  if (isFx) {
    return {
      type: "fdc3.instrument",
      name: `${clean} Spot Exchange Rate`,
      id: {
        ticker: clean,
        RIC: `${clean.replace("/", "")}=`,
      },
      market: {
        MIC: "XOFF",
        name: "OTC FX Market",
      },
    };
  }

  return {
    type: "fdc3.instrument",
    name: clean,
    id: {
      ticker: clean,
      RIC: `${clean}.O`,
    },
    market: {
      MIC: "XNAS",
      name: "Nasdaq Global Market",
    },
  };
}

/**
 * Parse ticker from inbound FDC3 Instrument Context.
 */
export function fdc3ToInstrument(context: Fdc3Context): string | null {
  if (context.type !== "fdc3.instrument") return null;
  const inst = context as Fdc3InstrumentContext;
  return inst.id.ticker || inst.id.RIC?.replace("=", "") || inst.id.ISIN || null;
}
