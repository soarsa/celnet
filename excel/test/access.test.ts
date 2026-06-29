// User-permission (access) parity tests for `excel/src/contract/access.ts`.
//
// These assert the invariants the add-in gates affordances on: the action × asset
// membership test (`can`), the honest denial sentence (byte-identical to the GUI's
// `capabilityDenialTitle`), and the entry-point → capability map (the add-in's
// RFQ / book / contribute / CELNET.* affordances → their server-enforced
// capability). They are the source of truth the task pane + cell gates read.

import { describe, expect, it } from "vitest";

import {
  ALL_ENTRY_POINTS,
  CAPABILITY_ACTIONS,
  CAPABILITY_ASSETS,
  ENTRY_POINTS,
  can,
  capabilityDenialTitle,
  entryDenialTitle,
  entrySignInPrompt,
  type Capability,
} from "../src/contract/access";

const FX_PRICE: Capability = { action: "price", asset: "fx_options" };
const FX_EXEC: Capability = { action: "execute", asset: "fx_options" };
const FI_PRICE: Capability = { action: "price", asset: "fixed_income" };

describe("can — effective-set membership", () => {
  it("returns true only for a capability present in the set", () => {
    const caps = [FX_PRICE, FI_PRICE];
    expect(can(caps, "price", "fx_options")).toBe(true);
    expect(can(caps, "price", "fixed_income")).toBe(true);
    expect(can(caps, "execute", "fx_options")).toBe(false);
    expect(can(caps, "stream", "fx_options")).toBe(false);
  });

  it("an empty set denies everything (deny-by-default)", () => {
    for (const action of CAPABILITY_ACTIONS) {
      for (const asset of CAPABILITY_ASSETS) {
        expect(can([], action, asset)).toBe(false);
      }
    }
  });

  it("distinguishes the same action across asset classes", () => {
    expect(can([FX_EXEC], "execute", "fx_options")).toBe(true);
    expect(can([FX_EXEC], "execute", "fixed_income")).toBe(false);
  });
});

describe("capabilityDenialTitle — the honest denial sentence", () => {
  it("names the action and asset class in plain language", () => {
    expect(capabilityDenialTitle("execute", "fx_options")).toBe(
      "Your permissions don't allow executing FX-options trades.",
    );
    expect(capabilityDenialTitle("book", "fixed_income")).toBe(
      "Your permissions don't allow booking fixed-income positions.",
    );
    expect(capabilityDenialTitle("stream", "fx_options")).toBe(
      "Your permissions don't allow streaming live FX-options prices.",
    );
    expect(capabilityDenialTitle("price", "fixed_income")).toBe(
      "Your permissions don't allow requesting fixed-income prices.",
    );
  });

  it("produces a non-empty sentence for every action × asset", () => {
    for (const action of CAPABILITY_ACTIONS) {
      for (const asset of CAPABILITY_ASSETS) {
        const title = capabilityDenialTitle(action, asset);
        expect(title.startsWith("Your permissions don't allow ")).toBe(true);
        expect(title.endsWith(".")).toBe(true);
      }
    }
  });
});

describe("ENTRY_POINTS — affordance → capability map", () => {
  it("maps the task-pane dealing affordances to their capabilities (require sign-in)", () => {
    expect(ENTRY_POINTS.rfq).toMatchObject({
      action: "price",
      asset: "fx_options",
      surface: "taskpane",
      requiresSignIn: true,
    });
    expect(ENTRY_POINTS.book).toMatchObject({
      action: "execute",
      asset: "fx_options",
      surface: "taskpane",
      requiresSignIn: true,
    });
    expect(ENTRY_POINTS.contribute).toMatchObject({
      action: "price",
      asset: "fx_options",
      surface: "taskpane",
      requiresSignIn: true,
    });
  });

  it("maps the cell functions by asset class (anonymous price-preview preserved)", () => {
    expect(ENTRY_POINTS.price).toMatchObject({ action: "price", asset: "fx_options", requiresSignIn: false });
    expect(ENTRY_POINTS.greeks).toMatchObject({ action: "price", asset: "fx_options" });
    expect(ENTRY_POINTS.rfq_cell).toMatchObject({ action: "price", asset: "fx_options" });
    expect(ENTRY_POINTS.subscribe).toMatchObject({ action: "stream", asset: "fx_options" });
    expect(ENTRY_POINTS.series).toMatchObject({ action: "stream", asset: "fx_options" });
    expect(ENTRY_POINTS.rates).toMatchObject({ action: "price", asset: "fixed_income" });
    expect(ENTRY_POINTS.marksurface).toMatchObject({ action: "price", asset: "fx_options" });
    expect(ENTRY_POINTS.mark).toMatchObject({ action: "price", asset: "fx_options" });
  });

  it("every entry's id matches its key, action ∈ actions, asset ∈ assets", () => {
    for (const e of ALL_ENTRY_POINTS) {
      expect(ENTRY_POINTS[e.id]).toBe(e);
      expect(CAPABILITY_ACTIONS).toContain(e.action);
      expect(CAPABILITY_ASSETS).toContain(e.asset);
      expect(e.label.length).toBeGreaterThan(0);
      expect(e.signInLabel.length).toBeGreaterThan(0);
    }
  });

  it("only task-pane affordances require a signed-in identity", () => {
    for (const e of ALL_ENTRY_POINTS) {
      if (e.surface === "taskpane") expect(e.requiresSignIn).toBe(true);
      else expect(e.requiresSignIn).toBe(false);
    }
  });

  it("entryDenialTitle / entrySignInPrompt delegate to the right capability", () => {
    expect(entryDenialTitle("book")).toBe(capabilityDenialTitle("execute", "fx_options"));
    expect(entryDenialTitle("rates")).toBe(capabilityDenialTitle("price", "fixed_income"));
    expect(entrySignInPrompt("rfq")).toBe("Sign in to request dealer quotes.");
    expect(entrySignInPrompt("book")).toBe("Sign in to book a trade.");
  });
});
