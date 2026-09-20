/**
 * The ticket task-pane controller: binds the pure, headless ticket models to the
 * DOM and the shared WS connection (the ONE celnet-proto contract).
 *
 * It does NOT recompute prices — every number comes from the server over the WS
 * mirror, so the task-pane panel is bit-identical to the cell, the GUI, and the
 * SDK. The pane is class-aware: a trader walks an ASSET CLASS → a per-class
 * UNDERLIER → a PRICEABLE product ARM → its TERMS (the headless
 * {@link BuilderState} in `instrumentBuilder.ts`), then RFQs the whole ticket to
 * the ranked MULTI-DEALER panel ({@link fromMultiDealer} / {@link accept} in
 * `dealerPanel.ts`) and books a chosen dealer line on its own last-look window.
 * The Contribute panel stages a mark (idempotent) and commits it on explicit
 * confirmation (never on a recalc). The pane is "alive" — quote ticks flash the
 * touch tiles, the header pinwheel spins on each quote / heartbeats on a drop,
 * the active underlier breathes, and each panel line's last-look ring drains.
 */

import { getConnection, getSession } from "../functions/runtime";
import type { EntryPointId } from "../contract/access";
import { DEFAULT_CONVENTIONS } from "../functions/shaping";
import { getStaged, stageMark, markCommitted, markRejected } from "../functions/markStaging";
import { brokerQuoteSetToWire, ccyPairToWire, conventionsToWire } from "../contract/wsCodec";
import { smileModel } from "../contract/enums";
import {
  IDLE_MARK,
  canCommitMark,
  markCommittedState,
  markRejectedState,
  markStaged,
  type MarkState,
} from "./ticketModel";
import {
  ALL_ARMS,
  ASSET_CLASSES,
  INITIAL_BUILDER_STATE,
  availableArms,
  buildInstrument,
  selectArm,
  selectAssetClass,
  setNotional,
  setTenor,
  setTerms,
  updateUnderlier,
  type AssetClass,
  type BuilderState,
  type TermsRow,
} from "./instrumentBuilder";
import { priceability } from "./capability";
import {
  DEFAULT_WINDOW_SECONDS,
  accept,
  fromMultiDealer,
  lastLookRemaining,
  type DealerLine,
  type DealerPanelState,
} from "./dealerPanel";
import type { TermsCell } from "../functions/instrumentSpec";
import type { ProductKind } from "./capability";

/** Map the trader-facing arm name to the contract product-kind for `priceability()`. */
const ARM_TO_KIND: Readonly<Record<string, ProductKind>> = {
  VANILLA: "vanilla",
  STRATEGY: "strategy",
  RISKREVERSAL: "strategy",
  STRADDLE: "strategy",
  STRANGLE: "strategy",
  SEAGULL: "strategy",
  BARRIER: "singleBarrier",
  WINDOWBARRIER: "windowBarrier",
  DIGITAL: "digital",
  TOUCH: "touch",
  VARSWAP: "varianceSwap",
  VOLSWAP: "volatilitySwap",
  ASIAN: "asianOption",
  FORWARDSTART: "forwardStart",
  CLIQUET: "cliquet",
  QUANTO: "quanto",
  TARF: "tarf",
  PIVOT: "pivot",
  ACCUMULATOR: "accumulator",
  LOOKBACK: "lookback",
  AMERICAN: "american",
  BASKET: "basket",
  FORWARD: "fxForward",
  SWAP: "fxSwap",
  NDF: "ndf",
  PERPETUAL: "perpetualOption",
  FUTUREOPTION: "listedFutureOption",
};

/**
 * Sensible default terms (the `key value` rows) for an arm, so the terms editor
 * opens on a complete, priceable ticket per family. These mirror each family's
 * minimal required key set in `functions/instrumentSpec.ts FAMILIES`; the build
 * still delegates EVERY term to that one shaper (this only pre-fills the editor —
 * the trader edits freely, and the shaper is the single validation authority).
 */
const ARM_DEFAULT_TERMS: Readonly<Record<string, readonly TermsRow[]>> = {
  VANILLA: [["strike", "ATM"], ["callPut", "C"]],
  STRADDLE: [["legs", "C", "ATM", "BUY"], ["legs", "P", "ATM", "BUY"]],
  RISKREVERSAL: [["legs", "C", "25dC", "BUY"], ["legs", "P", "25dP", "SELL"]],
  STRANGLE: [["legs", "C", "25dC", "BUY"], ["legs", "P", "25dP", "BUY"]],
  BARRIER: [["strike", "ATM"], ["callPut", "C"], ["kind", "UP_AND_OUT"], ["barrier", "1.20"]],
  DIGITAL: [["strike", "ATM"], ["callPut", "C"], ["payout", "1"]],
  TOUCH: [["barrier", "1.20"], ["kind", "ONE_TOUCH"], ["payout", "1"]],
  ASIAN: [["callPut", "C"], ["strike", "ATM"], ["style", "AVERAGE_RATE"]],
  VARSWAP: [["strikeVol", "0.10"]],
  VOLSWAP: [["strikeVol", "0.10"]],
  FORWARD: [["rate", "1.10"], ["side", "BUY"]],
  SWAP: [["rate", "1.10"], ["nearSide", "BUY"]],
  NDF: [["rate", "1.10"], ["fixing", "WMR"], ["settlementCcy", "USD"], ["side", "BUY"]],
  PERPETUAL: [["strike", "1.10"], ["callPut", "C"]],
  FUTUREOPTION: [["strike", "1.10"], ["callPut", "C"], ["futureSymbol", "CLZ6"], ["futureExpiry", "0.5"]],
};

/** The class-conditional underlier field-group containers, one per asset class. */
const UNDERLIER_GROUPS: Readonly<Record<AssetClass, string>> = {
  FX: "und-fx",
  METAL: "und-fx",
  EQUITY: "und-equity",
  COMMODITY: "und-commodity",
  CRYPTO: "und-crypto",
};

function el<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing element #${id}`);
  return node as T;
}

function val(id: string): string {
  return el<HTMLInputElement>(id).value.trim();
}

function setState(id: string, text: string, tone: "" | "ok" | "warn" | "bad" = ""): void {
  const node = el(id);
  node.textContent = text;
  node.className = `state${tone ? " " + tone : ""}`;
}

/**
 * Parse the terms textarea into the order-free `TermsRow[]` the builder stores —
 * one row per non-blank line, `key value [value…]` split on whitespace. Numeric
 * value cells are coerced to numbers (so `strike 1.12` is `["strike", 1.12]`);
 * everything else stays a string. Blank lines are skipped. This is the SAME 2-D
 * key/value grammar `CELNET.INSTRUMENT` reads — the shaper owns all validation.
 */
function parseTerms(raw: string): TermsRow[] {
  const rows: TermsRow[] = [];
  for (const line of raw.split("\n")) {
    const cells = line.trim().split(/\s+/).filter((c) => c.length > 0);
    if (cells.length === 0) continue;
    const row: TermsCell[] = cells.map((c, i) => {
      if (i === 0) return c; // the key is always a string.
      const n = Number(c);
      return c !== "" && Number.isFinite(n) ? n : c;
    });
    rows.push(row as TermsRow);
  }
  return rows;
}

/** Render the stored `TermsRow[]` back into the textarea's line grammar. */
function termsToText(terms: readonly TermsRow[]): string {
  return terms.map((row) => row.map((c) => String(c ?? "")).join(" ")).join("\n");
}

function boot(): void {
  // The ONE shared-runtime connection + session (functions/runtime.ts): a sign-in
  // here installs the bearer token on the SAME transport the CELNET.* cells use, so
  // both surfaces authenticate as one identity (docs §3.2, single multiplexed session).
  const conn = getConnection();
  const session = getSession();
  const connPill = el("conn");
  const markSvg = document.querySelector<SVGElement>(".hdr .mark");

  let connected = false;
  conn.onState((open: boolean) => {
    connected = open;
    connPill.textContent = open ? "connected" : "disconnected";
    connPill.className = `pill ${open ? "pill-on" : "pill-off"}`;
    // Liveness on the pinwheel: spin on connect, calm heartbeat while down.
    if (markSvg) {
      markSvg.classList.remove("mark-spin");
      if (open) markSvg.classList.remove("mark-heartbeat");
      else markSvg.classList.add("mark-heartbeat");
    }
  });

  // ---- ticket builder state (class-aware) ---------------------------------
  let state: BuilderState = INITIAL_BUILDER_STATE;
  let panel: DealerPanelState | null = null;
  let lastTouch: { bid: number; offer: number } | null = null;
  let ringTimer: ReturnType<typeof setInterval> | null = null;
  let mark: MarkState = IDLE_MARK;

  const classSel = el<HTMLSelectElement>("rfq-class");
  const armSel = el<HTMLSelectElement>("rfq-arm");
  const tenorWrap = el("rfq-tenor-wrap");
  const termsArea = el<HTMLTextAreaElement>("rfq-terms");

  // Populate the asset-class selector once (the classes are static).
  for (const cls of ASSET_CLASSES) {
    const opt = document.createElement("option");
    opt.value = cls;
    opt.textContent = cls;
    classSel.append(opt);
  }

  // ---- pinwheel liveness on each quote tick -------------------------------
  const spinMark = (): void => {
    if (!markSvg) return;
    markSvg.classList.remove("mark-heartbeat");
    markSvg.classList.remove("mark-spin");
    void markSvg.getBoundingClientRect(); // force reflow → restart the animation.
    markSvg.classList.add("mark-spin");
  };
  markSvg?.addEventListener("animationend", () => markSvg.classList.remove("mark-spin"));

  // ---- the class-conditional underlier fields -----------------------------
  const renderUnderlierFields = (): void => {
    const active = UNDERLIER_GROUPS[state.assetClass];
    for (const group of new Set(Object.values(UNDERLIER_GROUPS))) {
      el(group).hidden = group !== active;
    }
    const u = state.underlier;
    // Reflect the stored per-class inputs into the visible fields (a class switch
    // is non-destructive — each class keeps its own text).
    setInput("und-pair", u.pair);
    setInput("und-eq-ticker", u.ticker);
    setInput("und-eq-venue", u.venue);
    setInput("und-eq-ccy", u.currency);
    setInput("und-cm-ticker", u.ticker);
    setInput("und-cm-ccy", u.currency);
    setInput("und-cx-base", u.base);
    setInput("und-cx-quote", u.quote);
    el<HTMLSelectElement>("und-cx-settle").value = u.settlement;
  };

  // ---- the priceable-arm selector (non-priceable arms dimmed) -------------
  const renderArms = (): void => {
    const priceable = new Set(availableArms(state.assetClass));
    armSel.replaceChildren();
    // Offer EVERY arm so the trader SEES the full catalogue, but disable (dim) the
    // ones not priceable on this class, annotated with the honest capability why —
    // the same boundary `shapeSpecInstrument`'s guard enforces, surfaced BEFORE
    // the request rather than after a server refusal.
    for (const arm of ALL_ARMS) {
      const opt = document.createElement("option");
      opt.value = arm;
      const ok = priceable.has(arm);
      opt.disabled = !ok;
      if (ok) {
        opt.textContent = arm;
      } else {
        const kind = ARM_TO_KIND[arm];
        const verdict = kind ? priceability(kind, state.assetClass) : "priceable";
        opt.textContent = `${arm} — n/a on ${state.assetClass}`;
        if (verdict !== "priceable") opt.title = verdict.reason;
      }
      armSel.append(opt);
    }
    armSel.value = state.arm;
    // PERPETUAL is the no-expiry arm: hide the tenor input for it (a supplied tenor
    // is a typed build error — mirror that in the UI).
    tenorWrap.hidden = state.arm === "PERPETUAL";
  };

  const renderTerms = (): void => {
    termsArea.value = termsToText(state.terms);
  };

  const renderScalars = (): void => {
    setInput("rfq-tenor", state.tenor);
    setInput("rfq-notional", state.notional);
  };

  // Full ticket re-render (class fields + arm list + terms + scalars).
  const renderTicket = (): void => {
    classSel.value = state.assetClass;
    renderUnderlierFields();
    renderArms();
    renderTerms();
    renderScalars();
  };

  // ---- input wiring -------------------------------------------------------
  classSel.addEventListener("change", () => {
    state = selectAssetClass(state, classSel.value as AssetClass);
    renderTicket();
  });

  armSel.addEventListener("change", () => {
    try {
      state = selectArm(state, armSel.value);
    } catch {
      // The select only ever offers priceable arms; a guard rejection is ignored
      // and the prior arm restored (defensive — never leaves a bad arm selected).
      renderArms();
      return;
    }
    // Re-seed the terms editor with the new arm's sensible defaults so the ticket
    // stays complete across an arm switch (the trader then edits freely).
    const defaults = ARM_DEFAULT_TERMS[state.arm];
    if (defaults) state = setTerms(state, defaults.map((r) => [...r] as TermsRow));
    renderArms();
    renderTerms();
  });

  // Each per-class underlier field patches only its own slot via updateUnderlier.
  bindUnderlier("und-pair", (v) => updateUnderlier(state, { pair: v }));
  bindUnderlier("und-eq-ticker", (v) => updateUnderlier(state, { ticker: v }));
  bindUnderlier("und-eq-venue", (v) => updateUnderlier(state, { venue: v }));
  bindUnderlier("und-eq-ccy", (v) => updateUnderlier(state, { currency: v }));
  bindUnderlier("und-cm-ticker", (v) => updateUnderlier(state, { ticker: v }));
  bindUnderlier("und-cm-ccy", (v) => updateUnderlier(state, { currency: v }));
  bindUnderlier("und-cx-base", (v) => updateUnderlier(state, { base: v }));
  bindUnderlier("und-cx-quote", (v) => updateUnderlier(state, { quote: v }));
  el<HTMLSelectElement>("und-cx-settle").addEventListener("change", (e) => {
    const v = (e.target as HTMLSelectElement).value as BuilderState["underlier"]["settlement"];
    state = updateUnderlier(state, { settlement: v });
  });

  el<HTMLInputElement>("rfq-tenor").addEventListener("input", (e) => {
    state = setTenor(state, (e.target as HTMLInputElement).value);
  });
  el<HTMLInputElement>("rfq-notional").addEventListener("input", (e) => {
    state = setNotional(state, (e.target as HTMLInputElement).value);
  });
  termsArea.addEventListener("input", () => {
    state = setTerms(state, parseTerms(termsArea.value));
  });

  function bindUnderlier(id: string, fn: (v: string) => BuilderState): void {
    el<HTMLInputElement>(id).addEventListener("input", (e) => {
      state = fn((e.target as HTMLInputElement).value);
    });
  }

  // ---- the active two-way (touch best of the panel), flashes on tick ------
  const flashTile = (id: string, dir: "up" | "down"): void => {
    const node = el(id);
    node.classList.remove("quote-flash-up", "quote-flash-dn");
    void node.offsetWidth; // force reflow → restart the wash (PriceTile idiom).
    node.classList.add(dir === "up" ? "quote-flash-up" : "quote-flash-dn");
    node.dataset.dir = dir === "up" ? "up" : "down";
  };

  const renderTouch = (touch: { bid: number; offer: number } | null): void => {
    const bidEl = el("rfq-bid");
    const offerEl = el("rfq-offer");
    if (!touch) {
      bidEl.textContent = "—";
      offerEl.textContent = "—";
      lastTouch = null;
      return;
    }
    if (lastTouch) {
      if (touch.bid > lastTouch.bid) flashTile("rfq-bid", "up");
      else if (touch.bid < lastTouch.bid) flashTile("rfq-bid", "down");
      if (touch.offer > lastTouch.offer) flashTile("rfq-offer", "up");
      else if (touch.offer < lastTouch.offer) flashTile("rfq-offer", "down");
    }
    bidEl.textContent = touch.bid.toFixed(5);
    offerEl.textContent = touch.offer.toFixed(5);
    lastTouch = touch;
  };

  // The touch tiles show the best bid / best offer line of the panel (the server's
  // marked winners), so the headline two-way tracks the keenest dealer on each side.
  const touchOf = (p: DealerPanelState): { bid: number; offer: number } | null => {
    const first = p.lines[0];
    if (!first) return null;
    const bestBid = p.lines.find((l) => l.bestBid) ?? first;
    const bestOffer = p.lines.find((l) => l.bestOffer) ?? first;
    return { bid: bestBid.price.bid, offer: bestOffer.price.offer };
  };

  // ---- the ranked dealer panel rows ---------------------------------------
  const book = (line: DealerLine, side: "BUY" | "SELL"): void => {
    void (async () => {
      if (!panel) return;
      if (!session.canEntry("book")) {
        setState("trade-state", session.entryDenialReason("book"), "bad");
        return;
      }
      try {
        setState("trade-state", `accepting ${side} on ${line.lpId}…`, "warn");
        const exec = await accept(panel, line.lpId, side, conn);
        setState(
          "trade-state",
          `EXECUTED ${side} on ${line.lpId} @ ${exec.tradedPremium.toFixed(5)} — exec ${exec.executionId.toString()}`,
          "ok",
        );
      } catch (err) {
        setState("trade-state", `REJECTED — ${message(err)}`, "bad");
      }
    })();
  };

  const renderPanel = (): void => {
    const list = el<HTMLUListElement>("dealer-panel");
    list.replaceChildren();
    if (!panel) return;
    for (const line of panel.lines) {
      list.append(dealerRow(line, book));
    }
  };

  // The per-row last-look rings drain on a clock tick; an elapsed line disables
  // its side buttons (never book on an expired window — the contract's deadline).
  const tickRings = (): void => {
    if (!panel) return;
    const nowNanos = BigInt(Date.now()) * 1_000_000n;
    // The execute (book) capability gates every side button — a denied/anonymous
    // caller sees the rows but cannot click-to-trade (disabled + explained), never
    // a silent no-op. Re-read each tick so a sign-in/out flips the rows live.
    const canBook = session.canEntry("book");
    const bookReason = session.entryDenialReason("book");
    let anyLive = false;
    for (const line of panel.lines) {
      const frac = lastLookRemaining(line, nowNanos, DEFAULT_WINDOW_SECONDS);
      const row = document.getElementById(rowId(line.lpId));
      if (!row) continue;
      const ring = row.querySelector<HTMLElement>(".ring");
      if (ring) {
        ring.style.setProperty("--ll-frac", frac.toFixed(4));
        ring.classList.toggle("lastlook-ring--warn", frac > 0 && frac < 0.1875); // < ~1.5s of 8s.
        ring.classList.toggle("lastlook-ring--expired", frac <= 0);
      }
      const expired = frac <= 0;
      row.classList.toggle("expired", expired);
      for (const btn of row.querySelectorAll<HTMLButtonElement>(".side-btn")) {
        btn.disabled = expired || !connected || !canBook;
        btn.title = canBook ? "" : bookReason;
      }
      if (!expired) anyLive = true;
    }
    if (!anyLive && ringTimer !== null) {
      clearInterval(ringTimer);
      ringTimer = null;
      setState("rfq-lastlook", "all dealer windows elapsed — re-request", "warn");
    }
  };

  // ---- request the ranked multi-dealer panel ------------------------------
  el("rfq-go").addEventListener("click", () => {
    void (async () => {
      if (!session.canEntry("rfq")) {
        setState("rfq-state", session.entryDenialReason("rfq"), "bad");
        return;
      }
      const built = buildInstrument(state);
      if (!built.ok) {
        // A trader mistake the client can name first (a missing term, an FX-only
        // arm on a cross-asset underlier, a blank pair) — surfaced inline, never a
        // server round-trip for a mistake we already know.
        setState("rfq-state", built.error, "bad");
        return;
      }
      try {
        setState("rfq-state", "fanning out to dealers…", "warn");
        // One key per ticket: the server's accept is request-matched, so the SAME
        // key the panel carries is echoed when the trader books a line.
        const idempotencyKey = `taskpane-rfq:${Date.now()}`;
        const result = await conn.requestMultiDealerQuote(
          built.instrument,
          DEFAULT_CONVENTIONS,
          idempotencyKey,
        );
        panel = fromMultiDealer(result);
        renderPanel();
        renderTouch(touchOf(panel));
        // Slide the headline two-way in on a fresh panel (one-shot quote-appear).
        const twoWay = el("rfq-bid").closest(".two-way");
        if (twoWay) {
          twoWay.classList.remove("quote-appear");
          void (twoWay as HTMLElement).offsetWidth;
          twoWay.classList.add("quote-appear");
          twoWay.addEventListener(
            "animationend",
            () => twoWay.classList.remove("quote-appear"),
            { once: true },
          );
        }
        spinMark();
        setState(
          "rfq-state",
          `${panel.lines.length} dealer${panel.lines.length === 1 ? "" : "s"} · q-${panel.quoteId.toString()}`,
          "ok",
        );
        setState("rfq-lastlook", "live — click a price to book on its window", "ok");
        // Drive the draining last-look rings (and side-button enablement) ~30 fps.
        if (ringTimer === null) ringTimer = setInterval(tickRings, 33);
        tickRings();
      } catch (err) {
        setState("rfq-state", message(err), "bad");
      }
    })();
  });

  // ---- active-underlier brand pulse on the class label --------------------
  // The class selector breathes coral while a complete, priceable ticket is held,
  // signalling the live underlier without distraction (gated by reduced-motion).
  const refreshPulse = (): void => {
    const ok = buildInstrument(state).ok;
    classSel.classList.toggle("brand-pulse", ok);
  };
  classSel.addEventListener("change", refreshPulse);
  termsArea.addEventListener("input", refreshPulse);
  for (const g of new Set(Object.values(UNDERLIER_GROUPS))) {
    el(g).addEventListener("input", refreshPulse);
  }

  // ---- contribute-mark flow (unchanged two-phase stage → confirm) ---------
  el("mk-stage").addEventListener("click", () => {
    void (async () => {
      if (!session.canEntry("contribute")) {
        setState("mk-state", session.entryDenialReason("contribute"), "bad");
        return;
      }
      try {
        const status = await stageMark(conn, {
          pair: val("mk-pair"),
          tenor: val("mk-tenor"),
          pillar: val("mk-pillar"),
          vol: Number(val("mk-vol")),
          comment: val("mk-comment"),
          model: val("mk-model"),
        });
        mark = markStaged(status.stagingId);
        (el("mk-commit") as HTMLButtonElement).disabled = !canCommitMark(mark);
        setState("mk-state", "staged — confirm to contribute", "warn");
        el("mk-pending").textContent = `PENDING ${status.stagingId}`;
      } catch (err) {
        setState("mk-state", message(err), "bad");
      }
    })();
  });

  el("mk-commit").addEventListener("click", () => {
    void (async () => {
      if (!canCommitMark(mark)) return;
      if (!session.canEntry("contribute")) {
        setState("mk-state", session.entryDenialReason("contribute"), "bad");
        return;
      }
      try {
        setState("mk-state", "contributing…", "warn");
        // The commit: a single broker-quote-set mark for the (pair, tenor), with
        // the declared convention checked server-side (normalize cross-check). A
        // material mismatch is rejected, never silently corrupting the surface.
        // Carry the model the trader selected at stage time into the commit, so
        // the surface is calibrated with the chosen family (VV/SABR/SVI/SSVI) on
        // the ONE `mark_surface` contract path (its `smile_model` selector).
        const stagedModel = getStaged(mark.stagingId)?.model ?? "MARKET_HEDGE";
        const reply = await conn.markSurface({
          pair: ccyPairToWire(parsePairLoose(val("mk-pair"))),
          broker_quotes: [
            brokerQuoteSetToWire({
              tenorYears: tenorYearsLoose(val("mk-tenor")),
              atmVol: Number(val("mk-vol")),
              rr25: 0,
              bf25: 0,
              rr10: 0,
              bf10: 0,
              hasTenDelta: false,
            }),
          ],
          conventions: conventionsToWire(DEFAULT_CONVENTIONS),
          smile_model: smileModel.toWire(stagedModel),
        });
        const surfaceVersion = typeof reply["surface_version"] === "number"
          ? BigInt(Math.trunc(reply["surface_version"] as number))
          : 0n;
        markCommitted(mark.stagingId, surfaceVersion, "committed");
        mark = markCommittedState(mark, surfaceVersion, mark.stagingId);
        (el("mk-commit") as HTMLButtonElement).disabled = true;
        setState("mk-state", mark.status, "ok");
        el("mk-pending").textContent = mark.status;
      } catch (err) {
        markRejected(mark.stagingId, message(err));
        mark = markRejectedState(mark, message(err));
        setState("mk-state", mark.status, "bad");
        el("mk-pending").textContent = mark.status;
      }
    })();
  });

  // ---- sign-in / capability gating ----------------------------------------
  // Externalized Enterprise SSO / Office 365 pass-through over the shared transport.
  // The captured effective capabilities DISABLE-gate dealing affordances by action × asset.
  // Anonymous keeps the price-preview workflow; dealing controls require a
  // signed-in identity (the "sign in to deal" posture). The server still enforces.
  const signInBtn = el<HTMLButtonElement>("auth-signin");
  const signOutBtn = el<HTMLButtonElement>("auth-signout");

  /**
   * Resolve an external authentication bearer token from Office SSO, URL query
   * parameters, host injection, or container storage.
   */
  async function resolveExternalToken(): Promise<string | null> {
    // 1. Office JS native Single Sign-On (Office.auth.getAccessToken)
    const officeGlobal = (window as unknown as { Office?: { auth?: { getAccessToken: (opts?: unknown) => Promise<string> } } }).Office;
    if (officeGlobal?.auth?.getAccessToken) {
      try {
        const token = await officeGlobal.auth.getAccessToken({
          allowSignInPrompt: true,
          forMSGraphAccess: false,
        });
        if (token && token.length > 20) return token;
      } catch (e) {
        console.warn("[CelNet Excel] Office.auth.getAccessToken prompt fallback:", e);
      }
    }

    // 2. URL search parameters (?token=... or ?jwt=...)
    try {
      const urlParams = new URLSearchParams(window.location.search);
      const tokenParam = urlParams.get("token") || urlParams.get("jwt") || urlParams.get("access_token");
      if (tokenParam && tokenParam.length > 20) return tokenParam;
    } catch {}

    // 3. Injected host global
    const w = window as unknown as { __CELNET_AUTH_TOKEN__?: string; __DESKMODAL_AUTH_TOKEN__?: string };
    if (w.__CELNET_AUTH_TOKEN__) return w.__CELNET_AUTH_TOKEN__;
    if (w.__DESKMODAL_AUTH_TOKEN__) return w.__DESKMODAL_AUTH_TOKEN__;

    // 4. Local storage or environment fallback
    try {
      const stored = localStorage.getItem("celnet.excel.auth_token");
      if (stored && stored.length > 20) return stored;
    } catch {}

    return null;
  }

  /** Disable + EXPLAIN one button by its entry-point capability (never hide, never silent). */
  const gateButton = (id: EntryPointId, btnId: string, alsoDisabled = false): void => {
    const btn = el<HTMLButtonElement>(btnId);
    const allowed = session.canEntry(id);
    btn.disabled = alsoDisabled || !allowed;
    btn.title = allowed ? "" : session.entryDenialReason(id);
  };

  const applyGating = (): void => {
    gateButton("rfq", "rfq-go");
    gateButton("contribute", "mk-stage");
    gateButton("contribute", "mk-commit", !canCommitMark(mark));
    // The book (execute) buttons live in the dynamic panel rows; tickRings re-applies
    // their gating on its clock (and immediately here on a sign-in/out).
    tickRings();
  };

  const renderAuth = (): void => {
    const user = session.currentUser();
    el("auth-signedout").hidden = user !== null;
    el("auth-signedin").hidden = user === null;
    if (user) {
      const expired = session.isExpired();
      el("auth-who").textContent = expired
        ? `${user.displayName} — session expired, sign in again`
        : `${user.displayName} · ${user.role}`;
      const caps = el<HTMLUListElement>("auth-caps");
      caps.replaceChildren();
      for (const cap of session.effectiveCapabilities()) {
        const li = document.createElement("li");
        li.textContent = `${cap.action} · ${cap.asset}`;
        caps.append(li);
      }
    }
    applyGating();
  };

  const handleSignIn = async (): Promise<void> => {
    signInBtn.disabled = true;
    setState("auth-state", "connecting to enterprise SSO…", "warn");
    try {
      const token = await resolveExternalToken();
      if (token) {
        await session.signInWithToken(token);
        setState("auth-state", "", "");
        return;
      }

      // If no token acquired from SSO, provide clear feedback
      setState("auth-state", "No active SSO token found in Office or Gateway.", "bad");
    } catch (err) {
      setState("auth-state", message(err), "bad");
    } finally {
      signInBtn.disabled = false;
    }
  };

  signInBtn.addEventListener("click", () => {
    void handleSignIn();
  });

  // Silent auto-pass-through on load if external token is present
  void (async () => {
    try {
      const token = await resolveExternalToken();
      if (token && !session.isSignedIn()) {
        await session.signInWithToken(token);
      }
    } catch (e) {
      console.warn("[CelNet Excel] Silent auto-pass-through:", e);
    }
  })();

  signOutBtn.addEventListener("click", () => {
    void (async () => {
      signOutBtn.disabled = true;
      try {
        await conn.logout().catch(() => false); // best-effort server invalidation
      } finally {
        session.signOut();
        setState("auth-state", "signed out", "");
        signOutBtn.disabled = false;
      }
    })();
  });

  // Re-gate live whenever the identity changes (the session is shared with cells).
  session.subscribe(() => renderAuth());
  renderAuth();

  renderTicket();
  renderTouch(null);
  refreshPulse();
}

/** Build one ranked-panel row: LP id + rank, BEST badges, two clickable side
 * prices (hit the bid / lift the offer), and the draining last-look ring. */
function dealerRow(line: DealerLine, book: (line: DealerLine, side: "BUY" | "SELL") => void): HTMLLIElement {
  const row = document.createElement("li");
  row.className = "dealer-row";
  row.id = rowId(line.lpId);

  const lp = document.createElement("div");
  lp.className = "lp";
  const name = document.createElement("span");
  name.textContent = line.lpId;
  lp.append(name);
  if (line.bestBid) lp.append(badge("bid", "best bid"));
  if (line.bestOffer) lp.append(badge("offer", "best offer"));

  const rank = document.createElement("div");
  rank.className = "rank";
  rank.textContent = `#${line.rank + 1}`;

  // SELL hits this dealer's BID; BUY lifts its OFFER. The button face carries the
  // price; clicking books that side on the line's own last-look window.
  const bidBtn = sideButton("bid", "Bid · sell", line.price.bid, () => book(line, "SELL"));
  const offerBtn = sideButton("offer", "Offer · buy", line.price.offer, () => book(line, "BUY"));

  const ring = document.createElement("div");
  ring.className = "ring lastlook-ring";
  ring.setAttribute("role", "img");
  ring.setAttribute("aria-label", `last-look window for ${line.lpId}`);
  ring.style.setProperty("--ll-frac", "1");

  row.append(lp, rank, bidBtn, offerBtn, ring);
  return row;
}

function badge(side: "bid" | "offer", label: string): HTMLSpanElement {
  const b = document.createElement("span");
  b.className = `badge ${side}`;
  b.textContent = side === "bid" ? "BID" : "OFFER";
  b.title = label;
  return b;
}

function sideButton(
  side: "bid" | "offer",
  label: string,
  price: number,
  onClick: () => void,
): HTMLButtonElement {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = `side-btn ${side}`;
  const lab = document.createElement("span");
  lab.className = "lab";
  lab.textContent = label;
  const v = document.createElement("span");
  v.textContent = price.toFixed(5);
  btn.append(lab, v);
  btn.setAttribute("aria-label", `${label} ${price.toFixed(5)}`);
  // Spring-pop ripple on click (celer-tokens .btn-clicked), then book.
  btn.addEventListener("mousedown", () => btn.classList.add("btn-clicked"));
  btn.addEventListener("animationend", () => btn.classList.remove("btn-clicked"));
  btn.addEventListener("click", onClick);
  return btn;
}

/** A DOM-safe row id keyed on the LP id (LP ids are `SYNTH-LP-k` / `NATIVE`). */
function rowId(lpId: string): string {
  return `dealer-${lpId.replace(/[^A-Za-z0-9_-]/g, "_")}`;
}

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function setInput(id: string, value: string): void {
  const node = document.getElementById(id);
  if (node instanceof HTMLInputElement) node.value = value;
}

// Loose parsers for the commit body (the strict ones throw on stage already).
function parsePairLoose(raw: string): { base: string; quote: string } {
  const s = raw.trim().toUpperCase().replace("/", "");
  return { base: s.slice(0, 3), quote: s.slice(3, 6) };
}
function tenorYearsLoose(raw: string): number {
  const s = raw.trim().toUpperCase();
  const m = /^(\d+)([WMY])$/.exec(s);
  if (!m) return 1;
  const n = Number(m[1]);
  return m[2] === "W" ? (n * 7) / 365 : m[2] === "M" ? n / 12 : n;
}

// Office.onReady fires when the host is initialized; under a plain browser preview
// (no Office host) fall back to DOMContentLoaded so the pane is still interactive.
const officeGlobal = (globalThis as unknown as {
  Office?: { onReady?: (cb: () => void) => void };
}).Office;
if (officeGlobal?.onReady) {
  officeGlobal.onReady(() => boot());
} else if (typeof document !== "undefined") {
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }
}
