// ONE CONTRACT — the add-in's USER-PERMISSION layer (the "who may act" model),
// the Excel-package port of the GUI's `gui/src/lib/capabilityMatrix.ts` gating
// half + `gui/src/hooks/useAuth.ts can(...)` semantics (CLAUDE.md rule 9,
// semantics-identical). This is DISTINCT from `src/taskpane/capability.ts`, which
// is the PRODUCT price-matrix (which product arm an asset class can price) — do
// not conflate the two: this module answers "is THIS signed-in user permitted to
// take action X on asset class Y", a per-user entitlement; that module answers "is
// product P priceable on class C", a static product fact.
//
// One capability is one ACTION on one ASSET CLASS. The effective set a user holds
// is the server-resolved `role bundle ∪ grants ∖ denies` (deny-wins), enumerated
// over every action × asset and delivered on `AuthService.Login`
// (`LoginResult.capabilities`); this client only TESTS membership against that set
// — the server still enforces every RPC. Labels are the canonical snake_case the
// wire carries (`celnet.wire.CapabilityDesc`); provenance is in comments only
// (guardrail #8). It mirrors the server's `celnet-entitlements::Capability` /
// `Action` / `AssetClass` algebra exactly, so a `false` here matches a server deny.

/**
 * One action a capability may authorize (`celnet.wire.CapabilityDesc.action`).
 * The full set, in canonical (server discriminant) order, is {@link CAPABILITY_ACTIONS}.
 */
export type CapabilityAction =
  | "view"
  | "price"
  | "quote_respond"
  | "rfq_respond"
  | "ioi_respond"
  | "stream"
  | "execute"
  | "book"
  | "administer";

/** The asset class a capability applies to (`celnet.wire.CapabilityDesc.asset`). */
export type CapabilityAsset = "fx_options" | "fixed_income";

/** One capability: an {@link CapabilityAction} on a {@link CapabilityAsset}. */
export interface Capability {
  readonly action: CapabilityAction;
  readonly asset: CapabilityAsset;
}

/** The full action set in canonical order (the server discriminant order). */
export const CAPABILITY_ACTIONS: readonly CapabilityAction[] = [
  "view",
  "price",
  "quote_respond",
  "rfq_respond",
  "ioi_respond",
  "stream",
  "execute",
  "book",
  "administer",
];

/** Both asset classes in canonical order. */
export const CAPABILITY_ASSETS: readonly CapabilityAsset[] = ["fx_options", "fixed_income"];

/**
 * A user's authority level (`celnet.wire.UserRole`). `TRADER` (the wire zero
 * default) is the dealing role; `ADMIN` additionally administers. A
 * defaulted/forgotten value can never grant administration.
 */
export type UserRole = "TRADER" | "ADMIN";

/**
 * A user account as exposed on the wire (`celnet.wire.UserDesc`) — carries NO
 * password material. `deskId` is omitted when the user is unassigned.
 */
export interface UserDesc {
  readonly id: string;
  readonly email: string;
  readonly displayName: string;
  readonly role: UserRole;
  readonly deskId?: string;
  readonly disabled: boolean;
}

/** The issued session on a successful login (`celnet.wire.LoginResponse`). */
export interface LoginResult {
  /** The opaque bearer token to present on subsequent RPCs (a secret). */
  readonly token: string;
  /** The authenticated user's profile. */
  readonly user: UserDesc;
  /** Absolute session expiry (epoch nanos); re-login is required past it. */
  readonly expiresNanos: bigint;
  /**
   * The caller's OWN fully-resolved effective capability set (`role bundle ∪
   * grants ∖ denies`, deny-wins, enumerated over every action × asset). The single
   * source for "what may THIS signed-in user do"; the server still enforces.
   */
  readonly capabilities: readonly Capability[];
}

/**
 * Membership test against a user's effective capability set: does `caps` contain
 * `action × asset`? This is the single selector the client gates affordances on —
 * the effective set is the server's authoritative `role bundle ∪ grants ∖ denies`
 * (`LoginResult.capabilities`), so a `false` here mirrors a server deny exactly.
 * An empty set denies everything (a coherent deny-by-default when the caller's
 * capabilities are unknown). UX-only: the server still enforces.
 */
export function can(
  caps: readonly Capability[],
  action: CapabilityAction,
  asset: CapabilityAsset,
): boolean {
  return caps.some((c) => c.action === action && c.asset === asset);
}

/** The asset-class adjective used in a human-readable denial tooltip. */
const ASSET_ADJECTIVE: Record<CapabilityAsset, string> = {
  fx_options: "FX-options",
  fixed_income: "fixed-income",
};

/** The gerund phrase for each action, completed with the asset adjective. */
const ACTION_PHRASE: Record<CapabilityAction, (asset: string) => string> = {
  view: (a) => `viewing ${a} data`,
  price: (a) => `requesting ${a} prices`,
  quote_respond: (a) => `responding to ${a} dealer quote requests`,
  rfq_respond: (a) => `responding to ${a} RFQs`,
  ioi_respond: (a) => `responding to ${a} IOIs`,
  stream: (a) => `streaming live ${a} prices`,
  execute: (a) => `executing ${a} trades`,
  book: (a) => `booking ${a} positions`,
  administer: () => `administering Celnet`,
};

/**
 * The explanatory tooltip shown on an affordance the signed-in user is NOT
 * permitted to use — e.g. "Your permissions don't allow executing FX-options
 * trades." Gated controls are DISABLED (never hidden) and carry this text so the
 * denial is discoverable and explained, not a silent grey-out. Byte-identical to
 * the GUI's `capabilityDenialTitle` so both clients speak with one voice.
 */
export function capabilityDenialTitle(
  action: CapabilityAction,
  asset: CapabilityAsset,
): string {
  return `Your permissions don't allow ${ACTION_PHRASE[action](ASSET_ADJECTIVE[asset])}.`;
}

// ---------------------------------------------------------------------------
// entry-point → capability mapping
//
// The capability algebra above is action × asset (the wire contract). The add-in,
// however, has concrete AFFORDANCES — the task-pane RFQ / book / contribute
// controls and the CELNET.* worksheet functions. This table is the pure, testable
// bridge: each affordance declares the single capability (action on asset) that
// gates it, mirroring the GUI's per-workspace `can("…", "…")` mapping:
//
//   * Ticket request-quote / PRICE / GREEKS / RFQ → price  · fx_options
//   * book (click-to-trade accept)                → execute · fx_options
//   * SUBSCRIBE / SERIES (live streams)           → stream  · fx_options
//   * contribute a mark / MARKSURFACE / MARK      → price   · fx_options
//   * RATES (OIS pricing) / RATESRISK (book risk) → price   · fixed_income
//   * RATESBOOK (book ledger) / INSTRUMENTS (ref) → view    · fixed_income
//
// Every FX-options PRODUCT (incl. the cross-asset equity/commodity/crypto leaves
// that book through the same QuoteService) maps to the `fx_options` capability
// asset; the rates/FI surfaces map to `fixed_income`.
// ---------------------------------------------------------------------------

/** The add-in surface an entry point lives on. */
export type EntrySurface = "taskpane" | "cell";

/** A stable id for one gated affordance. */
export type EntryPointId =
  // task-pane dealing affordances (require a signed-in identity)
  | "rfq"
  | "book"
  | "contribute"
  // worksheet custom functions (anonymous price-preview stays per existing UX)
  | "price"
  | "greeks"
  | "rfq_cell"
  | "subscribe"
  | "series"
  | "rates"
  | "bond"
  | "ratesrisk"
  | "ratesbook"
  | "curve"
  | "instruments"
  | "marksurface"
  | "mark";

/** One gated affordance and the capability that admits it. */
export interface EntryPoint {
  readonly id: EntryPointId;
  /** Human label for the affordance (used in diagnostics/tests). */
  readonly label: string;
  /** The surface the affordance lives on. */
  readonly surface: EntrySurface;
  /** The capability action that admits the affordance. */
  readonly action: CapabilityAction;
  /** The capability asset class the action applies to. */
  readonly asset: CapabilityAsset;
  /**
   * Whether the affordance requires a SIGNED-IN identity (the dealing posture):
   * `true` ⇒ disabled while anonymous with a "sign in to …" prompt; `false` ⇒ the
   * existing anonymous price-preview behaviour is preserved (gating only NARROWS a
   * real signed-in user, exactly like the GUI's permissive anonymous path).
   */
  readonly requiresSignIn: boolean;
  /** The short clause used in the "Sign in to <…>." prompt (dealing affordances). */
  readonly signInLabel: string;
}

/** THE entry-point → capability spec for the add-in. */
export const ENTRY_POINTS: Record<EntryPointId, EntryPoint> = {
  rfq: {
    id: "rfq",
    label: "Request quote (RFQ panel)",
    surface: "taskpane",
    action: "price",
    asset: "fx_options",
    requiresSignIn: true,
    signInLabel: "request dealer quotes",
  },
  book: {
    id: "book",
    label: "Book (click-to-trade)",
    surface: "taskpane",
    action: "execute",
    asset: "fx_options",
    requiresSignIn: true,
    signInLabel: "book a trade",
  },
  contribute: {
    id: "contribute",
    label: "Contribute mark",
    surface: "taskpane",
    action: "price",
    asset: "fx_options",
    requiresSignIn: true,
    signInLabel: "contribute a mark",
  },
  price: {
    id: "price",
    label: "CELNET.PRICE",
    surface: "cell",
    action: "price",
    asset: "fx_options",
    requiresSignIn: false,
    signInLabel: "price",
  },
  greeks: {
    id: "greeks",
    label: "CELNET.GREEKS",
    surface: "cell",
    action: "price",
    asset: "fx_options",
    requiresSignIn: false,
    signInLabel: "price",
  },
  rfq_cell: {
    id: "rfq_cell",
    label: "CELNET.RFQ",
    surface: "cell",
    action: "price",
    asset: "fx_options",
    requiresSignIn: false,
    signInLabel: "request quotes",
  },
  subscribe: {
    id: "subscribe",
    label: "CELNET.SUBSCRIBE",
    surface: "cell",
    action: "stream",
    asset: "fx_options",
    requiresSignIn: false,
    signInLabel: "stream prices",
  },
  series: {
    id: "series",
    label: "CELNET.SERIES",
    surface: "cell",
    action: "stream",
    asset: "fx_options",
    requiresSignIn: false,
    signInLabel: "stream prices",
  },
  rates: {
    id: "rates",
    label: "CELNET.RATES",
    surface: "cell",
    action: "price",
    asset: "fixed_income",
    requiresSignIn: false,
    signInLabel: "price rates",
  },
  bond: {
    id: "bond",
    label: "CELNET.BOND",
    surface: "cell",
    action: "price",
    asset: "fixed_income",
    requiresSignIn: false,
    signInLabel: "price bonds",
  },
  ratesrisk: {
    id: "ratesrisk",
    label: "CELNET.RATESRISK",
    surface: "cell",
    action: "price",
    asset: "fixed_income",
    requiresSignIn: false,
    signInLabel: "aggregate rates risk",
  },
  ratesbook: {
    id: "ratesbook",
    label: "CELNET.RATESBOOK",
    surface: "cell",
    // A read of the server-owned rates BOOK (the read side maps to `view`, exactly
    // like the GUI's `readActions: ["view"]` on every fixed-income workspace).
    action: "view",
    asset: "fixed_income",
    requiresSignIn: false,
    signInLabel: "list the rates book",
  },
  curve: {
    id: "curve",
    label: "CELNET.CURVE",
    surface: "cell",
    action: "price",
    asset: "fixed_income",
    requiresSignIn: false,
    signInLabel: "bootstrap curves",
  },
  instruments: {
    id: "instruments",
    label: "CELNET.INSTRUMENTS",
    surface: "cell",
    // A read of the instrument reference-data roster (the `view` read action, like
    // the GUI Reference Data workspace; the server admits any authenticated caller).
    action: "view",
    asset: "fixed_income",
    requiresSignIn: false,
    signInLabel: "view reference data",
  },
  marksurface: {
    id: "marksurface",
    label: "CELNET.MARKSURFACE",
    surface: "cell",
    action: "price",
    asset: "fx_options",
    requiresSignIn: false,
    signInLabel: "contribute a mark",
  },
  mark: {
    id: "mark",
    label: "CELNET.MARK",
    surface: "cell",
    action: "price",
    asset: "fx_options",
    requiresSignIn: false,
    signInLabel: "contribute a mark",
  },
};

/** Every entry point in declaration order (the enumeration domain for tests). */
export const ALL_ENTRY_POINTS: readonly EntryPoint[] = Object.values(ENTRY_POINTS);

/** The capability denial tooltip for an entry point (delegates to {@link capabilityDenialTitle}). */
export function entryDenialTitle(id: EntryPointId): string {
  const e = ENTRY_POINTS[id];
  return capabilityDenialTitle(e.action, e.asset);
}

/** The "sign in to …" prompt for an entry point that requires an identity. */
export function entrySignInPrompt(id: EntryPointId): string {
  return `Sign in to ${ENTRY_POINTS[id].signInLabel}.`;
}
