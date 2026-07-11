/**
 * AdminWorkspace — the tabbed admin screen + MANY-TO-MANY desk membership.
 *
 * Drives the workspace with `useApp`/`useAdmin` mocked so we inject a fixed roster
 * and observe the mutations. Covers: the sign-in gate, the four section TABS (one
 * pane at a time, Users default), the Users row layout (role badge + capability
 * chips + the four action controls all present), and the desk-membership cell —
 * the three states (All desks / a desk set / deskless) render, editing sends
 * `setUserDesks(deskIds[], allDesks)`, and a rejected change surfaces inline.
 */

import { render, screen, fireEvent, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { UserDesc } from "../src/data/contract";

const state: { app: unknown; admin: unknown } = { app: null, admin: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));
vi.mock("../src/hooks/useAdmin", () => ({ useAdmin: () => state.admin }));

import { AdminWorkspace } from "../src/workspaces/AdminWorkspace";

function trader(overrides: Partial<UserDesc> = {}): UserDesc {
  return {
    id: "u1",
    email: "trader@celnet.com",
    displayName: "Jane Trader",
    role: "TRADER",
    deskIds: [],
    allDesks: false,
    disabled: false,
    ...overrides,
  };
}

function makeAdmin(overrides: Record<string, unknown> = {}) {
  return {
    users: [trader()],
    desks: [
      { id: "g10", name: "G10 Options" },
      { id: "em", name: "EM Rates" },
    ],
    entities: [],
    books: [],
    isLoading: false,
    error: null,
    refetch: vi.fn(async () => {}),
    createUser: vi.fn(),
    updateUser: vi.fn(),
    setUserDesks: vi.fn(async () => {}),
    deleteUser: vi.fn(),
    resetPassword: vi.fn(),
    createDesk: vi.fn(),
    updateDesk: vi.fn(async () => ({ id: "g10", name: "G10 Vol" })),
    deleteDesk: vi.fn(),
    createEntity: vi.fn(),
    updateEntity: vi.fn(),
    deleteEntity: vi.fn(),
    createBook: vi.fn(),
    updateBook: vi.fn(),
    deleteBook: vi.fn(),
    ...overrides,
  };
}

function makeApp(isAdmin: boolean, user: { id: string; email: string } | null) {
  return {
    transport: {},
    auth: { isAdmin, user, busy: false, logout: vi.fn(async () => {}) },
    setSignInOpen: vi.fn(),
  };
}

/** Sign in as the seeded admin with one trader + two desks, and render. */
function renderAsAdmin(overrides: Record<string, unknown> = {}) {
  state.app = makeApp(true, { id: "admin1", email: "admin@celnet.com" });
  state.admin = makeAdmin(overrides);
  return render(<AdminWorkspace />);
}

/** The Users roster row for the seeded trader (the row holding its Edit button). */
function traderRow(): HTMLElement {
  return screen
    .getAllByRole("row")
    .find((r) => within(r).queryByRole("button", { name: "Permissions" }) !== null)!;
}

beforeEach(() => {
  state.app = null;
  state.admin = null;
  vi.clearAllMocks();
});

describe("AdminWorkspace — sign-in gate", () => {
  it("shows a sign-in card (not the tabs) for an anonymous session", () => {
    state.app = makeApp(false, null);
    state.admin = makeAdmin();
    render(<AdminWorkspace />);
    expect(screen.getByRole("button", { name: "Sign in" })).toBeInTheDocument();
    expect(screen.queryByRole("tablist", { name: "administration sections" })).toBeNull();
  });
});

describe("AdminWorkspace — the four section tabs", () => {
  it("renders the four tabs, defaults to Users, and switches one pane at a time", () => {
    renderAsAdmin();
    const tabs = screen.getByRole("tablist", { name: "administration sections" });
    for (const label of ["Users", "Desks", "Legal Entities", "Netting Books"]) {
      expect(within(tabs).getByRole("tab", { name: label })).toBeInTheDocument();
    }
    // Users is the default selected pane.
    expect(within(tabs).getByRole("tab", { name: "Users" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("heading", { name: "Users" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Desks" })).toBeNull();

    // Switching swaps the pane (only one section mounted at a time).
    fireEvent.click(within(tabs).getByRole("tab", { name: "Desks" }));
    expect(screen.getByRole("heading", { name: "Desks" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Users" })).toBeNull();
    expect(screen.getByRole("button", { name: "Add desk" })).toBeInTheDocument();

    fireEvent.click(within(tabs).getByRole("tab", { name: "Legal Entities" }));
    expect(screen.getByRole("heading", { name: "Legal entities" })).toBeInTheDocument();

    fireEvent.click(within(tabs).getByRole("tab", { name: "Netting Books" }));
    expect(screen.getByRole("heading", { name: "Netting books" })).toBeInTheDocument();
  });
});

describe("AdminWorkspace — Users row layout", () => {
  it("renders the role badge, capability chips and all four action controls in one row", () => {
    renderAsAdmin();
    const row = traderRow();
    expect(within(row).getByText("Trader")).toBeInTheDocument();
    expect(within(row).getByText(/FX \d+\/\d+/)).toBeInTheDocument();
    for (const name of ["Edit", "Reset password", "Permissions", "Delete"]) {
      expect(within(row).getByRole("button", { name })).toBeInTheDocument();
    }
  });
});

describe("AdminWorkspace — desk membership renders the three states", () => {
  it("a deskless trader shows the 'receives no quotes' marker with nothing checked", () => {
    renderAsAdmin();
    const row = traderRow();
    expect(within(row).getByText("receives no quotes")).toBeInTheDocument();
    expect(
      within(row).getByRole("checkbox", { name: "All desks for trader@celnet.com" }),
    ).not.toBeChecked();
    expect(within(row).getByRole("checkbox", { name: "G10 Options" })).not.toBeChecked();
  });

  it("a set-membership trader shows the desk chips and checks the matching boxes", () => {
    renderAsAdmin({ users: [trader({ deskIds: ["g10", "em"] })] });
    const row = traderRow();
    expect(within(row).getByRole("checkbox", { name: "G10 Options" })).toBeChecked();
    expect(within(row).getByRole("checkbox", { name: "EM Rates" })).toBeChecked();
    // The at-a-glance summary chips name both desks (label text + chip ⇒ ≥2 each).
    expect(within(row).getAllByText("G10 Options").length).toBeGreaterThanOrEqual(2);
    expect(within(row).queryByText("receives no quotes")).toBeNull();
  });

  it("an all-desks trader checks the toggle, hides the desk list, and shows the All-desks chip", () => {
    renderAsAdmin({ users: [trader({ allDesks: true })] });
    const row = traderRow();
    expect(
      within(row).getByRole("checkbox", { name: "All desks for trader@celnet.com" }),
    ).toBeChecked();
    // No per-desk checkboxes while All-desks is on.
    expect(within(row).queryByRole("checkbox", { name: "G10 Options" })).toBeNull();
    // The summary shows the "All desks" chip (toggle label + chip ⇒ ≥2).
    expect(within(row).getAllByText("All desks").length).toBeGreaterThanOrEqual(2);
  });
});

describe("AdminWorkspace — editing membership sends setUserDesks", () => {
  it("checking a desk sends the new desk_ids set with allDesks=false", () => {
    const setUserDesks = vi.fn(async () => {});
    renderAsAdmin({ users: [trader({ deskIds: ["g10"] })], setUserDesks });
    const row = traderRow();
    fireEvent.click(within(row).getByRole("checkbox", { name: "EM Rates" }));
    expect(setUserDesks).toHaveBeenCalledWith("u1", ["g10", "em"], false);
  });

  it("unchecking the last desk sends an empty set (deskless)", () => {
    const setUserDesks = vi.fn(async () => {});
    renderAsAdmin({ users: [trader({ deskIds: ["g10"] })], setUserDesks });
    const row = traderRow();
    fireEvent.click(within(row).getByRole("checkbox", { name: "G10 Options" }));
    expect(setUserDesks).toHaveBeenCalledWith("u1", [], false);
  });

  it("toggling All desks sends allDesks=true with an empty desk set", () => {
    const setUserDesks = vi.fn(async () => {});
    renderAsAdmin({ users: [trader({ deskIds: ["g10"] })], setUserDesks });
    const row = traderRow();
    fireEvent.click(within(row).getByRole("checkbox", { name: "All desks for trader@celnet.com" }));
    expect(setUserDesks).toHaveBeenCalledWith("u1", [], true);
  });

  it("surfaces an inline error in the row when the change is rejected", async () => {
    const setUserDesks = vi.fn(async () => {
      throw new Error("permission_denied");
    });
    renderAsAdmin({ users: [trader({ deskIds: ["g10"] })], setUserDesks });
    const row = traderRow();
    fireEvent.click(within(row).getByRole("checkbox", { name: "EM Rates" }));
    expect(await within(row).findByRole("alert")).toHaveTextContent("permission_denied");
  });
});
