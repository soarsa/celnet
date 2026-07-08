/**
 * AdminWorkspace — the admin gate + the inline desk picker.
 *
 * (a) A non-admin session renders the sign-in / insufficient-role gate and NO
 *     desk `<select>` (admin gating — the desk control is admin-only).
 * (b) An admin changing a user's inline Desk `<select>` calls `assignDesk(userId,
 *     deskId)`, and a REJECTED assignment surfaces a per-row inline error.
 *
 * `useApp` and `useAdmin` are mocked so the test drives the workspace's own
 * routing/gating logic in isolation; deny-wins capability algebra is covered in
 * capabilityMatrix.test.ts.
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { UserDesc } from "../src/data/contract";

// Mutable state the hoisted mocks read (set per test before render).
const state = vi.hoisted(() => ({
  app: null as unknown,
  admin: null as unknown,
}));

vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));
vi.mock("../src/hooks/useAdmin", () => ({ useAdmin: () => state.admin }));

import { AdminWorkspace } from "../src/workspaces/AdminWorkspace";

function trader(overrides: Partial<UserDesc> = {}): UserDesc {
  return {
    id: "u1",
    email: "trader@celnet.com",
    displayName: "Jane Trader",
    role: "TRADER",
    disabled: false,
    ...overrides,
  };
}

function makeAdmin(overrides: Record<string, unknown> = {}) {
  return {
    users: [trader()],
    desks: [{ id: "g10", name: "G10 Options" }],
    entities: [],
    books: [],
    isLoading: false,
    error: null,
    refetch: vi.fn(async () => {}),
    createUser: vi.fn(),
    updateUser: vi.fn(),
    assignDesk: vi.fn(async () => {}),
    deleteUser: vi.fn(),
    resetPassword: vi.fn(),
    createDesk: vi.fn(),
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

describe("AdminWorkspace — admin gate", () => {
  it("renders the sign-in gate and NO desk select for a non-admin session", () => {
    state.app = makeApp(false, null);
    state.admin = makeAdmin();
    render(<AdminWorkspace />);

    expect(screen.getByRole("heading", { name: "Administration" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign in" })).toBeInTheDocument();
    // The inline desk control must be absent behind the admin gate.
    expect(screen.queryByRole("combobox", { name: /Desk for/ })).toBeNull();
  });
});

describe("AdminWorkspace — inline desk assignment", () => {
  it("calls assignDesk(userId, deskId) when an admin changes the desk select", async () => {
    const assignDesk = vi.fn(async () => {});
    state.app = makeApp(true, { id: "admin1", email: "admin@celnet.com" });
    state.admin = makeAdmin({ assignDesk });
    render(<AdminWorkspace />);

    const select = screen.getByRole("combobox", { name: "Desk for trader@celnet.com" });
    fireEvent.change(select, { target: { value: "g10" } });
    expect(assignDesk).toHaveBeenCalledWith("u1", "g10");
  });

  it("surfaces a per-row inline error when assignDesk rejects", async () => {
    const assignDesk = vi.fn(async () => {
      throw new Error("permission_denied");
    });
    state.app = makeApp(true, { id: "admin1", email: "admin@celnet.com" });
    state.admin = makeAdmin({ assignDesk });
    render(<AdminWorkspace />);

    const select = screen.getByRole("combobox", { name: "Desk for trader@celnet.com" });
    fireEvent.change(select, { target: { value: "g10" } });
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("permission_denied"),
    );
  });
});
