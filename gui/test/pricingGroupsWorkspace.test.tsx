/**
 * PricingGroupsWorkspace — the admin drag-and-drop pricing-pipeline builder, whose
 * editor is now a DISMISSIBLE portal modal over the group list (it was previously an
 * always-visible inline pane with no way to close it). These tests gate that
 * open/close seam without a server, driving the workspace with `useApp` mocked:
 *
 *  (a) the base view is the group list — no dialog is mounted until you open one;
 *  (b) "+ New pricing group" opens the editor as a labelled `role="dialog"` modal;
 *  (c) selecting an existing group opens the modal titled for that group;
 *  (d) the modal closes via the X button, Esc, backdrop click, and Cancel/Close —
 *      without persisting;
 *  (e) a valid create Saves through the transport and then closes the modal;
 *  (f) all the editor behaviour still renders inside the modal (fields, mode tabs,
 *      the feature palette + canvas + preview once a custom pipeline is enabled).
 *
 * `useTour()` is a safe no-op here (no TourProvider is mounted), so the guided-tour
 * seam degrades to `activeTourId === null` exactly as in an isolated render.
 */

import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import type { PricingGroup } from "../src/data/contract";

const state: { app: unknown } = { app: null };
vi.mock("../src/app/AppContext", () => ({ useApp: () => state.app }));

import { PricingGroupsWorkspace } from "../src/workspaces/PricingGroupsWorkspace";
import { pricingGroupSpecToWire } from "../src/data/wsCodec";

function group(overrides: Partial<PricingGroup> = {}): PricingGroup {
  return {
    id: "GROUP-A",
    name: "GROUP-A",
    description: "",
    memberConnectionIds: [],
    memberUserIds: [],
    memberDesks: [],
    espPipeline: null,
    rfqPipeline: null,
    sharePipeline: false,
    enabled: true,
    pricingSourceMode: 0,
    bookSkewWeight: null,
    ...overrides,
  };
}

function makeApp(opts: {
  user: { id: string; email: string } | null;
  isAdmin: boolean;
  groups: PricingGroup[];
  createPricingGroup?: ReturnType<typeof vi.fn>;
  updatePricingGroup?: ReturnType<typeof vi.fn>;
}) {
  return {
    transport: {
      listPricingGroups: vi.fn(async () => opts.groups),
      listFixConnections: vi.fn(async () => []),
      listUsers: vi.fn(async () => []),
      listDesks: vi.fn(async () => []),
      createPricingGroup:
        opts.createPricingGroup ??
        vi.fn(async (g: PricingGroup) => ({ ...g, id: g.id || "MINTED" })),
      updatePricingGroup:
        opts.updatePricingGroup ?? vi.fn(async (_id: string, g: PricingGroup) => g),
      updatePricingGroupPipeline: vi.fn(async () => group()),
      deletePricingGroup: vi.fn(async () => undefined),
    },
    auth: {
      user: opts.user,
      isAdmin: opts.isAdmin,
      can: () => opts.isAdmin,
    },
    setSignInOpen: vi.fn(),
  };
}

const admin = { id: "admin", email: "admin@celnet.com" };

beforeEach(() => {
  state.app = null;
  vi.clearAllMocks();
});

afterEach(() => {
  cleanup();
});

describe("PricingGroupsWorkspace — base list view", () => {
  it("shows the group list and the New button, with NO editor modal mounted", async () => {
    state.app = makeApp({ user: admin, isAdmin: true, groups: [group()] });
    render(<PricingGroupsWorkspace />);

    expect(
      await screen.findByRole("button", { name: /New pricing group/i }),
    ).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: /GROUP-A/ })).toBeInTheDocument();
    // The editor is a modal — it must NOT be present until opened.
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("PricingGroupsWorkspace — opening the editor modal", () => {
  it('opens a labelled "New pricing group" modal from the New button', async () => {
    state.app = makeApp({ user: admin, isAdmin: true, groups: [group()] });
    render(<PricingGroupsWorkspace />);

    fireEvent.click(await screen.findByRole("button", { name: /New pricing group/i }));

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(within(dialog).getByRole("heading", { name: "New pricing group" })).toBeInTheDocument();
    // The full editor renders inside the modal.
    expect(within(dialog).getByLabelText(/^Name/)).toBeInTheDocument();
    expect(within(dialog).getByRole("tab", { name: /ESP/ })).toBeInTheDocument();
  });

  it("opens the modal titled for a selected existing group", async () => {
    state.app = makeApp({
      user: admin,
      isAdmin: true,
      groups: [group({ id: "GROUP-B", name: "GROUP-B" })],
    });
    render(<PricingGroupsWorkspace />);

    fireEvent.click(await screen.findByRole("button", { name: /GROUP-B/ }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("heading", { name: "GROUP-B" })).toBeInTheDocument();
  });

  it("reveals the feature palette + canvas + preview once a custom pipeline is enabled", async () => {
    state.app = makeApp({ user: admin, isAdmin: true, groups: [group()] });
    render(<PricingGroupsWorkspace />);
    fireEvent.click(await screen.findByRole("button", { name: /New pricing group/i }));
    const dialog = await screen.findByRole("dialog");

    // Palette/canvas/preview are gated behind the per-mode custom-pipeline toggle.
    expect(within(dialog).queryByTestId("pipeline-canvas")).toBeNull();
    fireEvent.click(within(dialog).getByLabelText(/Custom ESP pipeline/i));
    expect(within(dialog).getByTestId("pipeline-canvas")).toBeInTheDocument();
    expect(within(dialog).getByTestId("preview-waterfall")).toBeInTheDocument();
    expect(within(dialog).getByTestId("palette-TIERING")).toBeInTheDocument();
  });
});

describe("PricingGroupsWorkspace — closing the modal", () => {
  async function openCreate() {
    render(<PricingGroupsWorkspace />);
    fireEvent.click(await screen.findByRole("button", { name: /New pricing group/i }));
    return screen.findByRole("dialog");
  }

  it("closes via the X button", async () => {
    state.app = makeApp({ user: admin, isAdmin: true, groups: [group()] });
    await openCreate();
    fireEvent.click(screen.getByRole("button", { name: /Close pricing group editor/i }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("closes via Esc", async () => {
    state.app = makeApp({ user: admin, isAdmin: true, groups: [group()] });
    await openCreate();
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("closes via a backdrop (scrim) click", async () => {
    state.app = makeApp({ user: admin, isAdmin: true, groups: [group()] });
    const dialog = await openCreate();
    // The scrim is the dialog's parent (role=presentation); mousedown on it closes.
    const scrim = dialog.parentElement as HTMLElement;
    fireEvent.mouseDown(scrim);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("closes via the Cancel action without persisting", async () => {
    const createPricingGroup = vi.fn(async (g: PricingGroup) => ({ ...g, id: "X" }));
    state.app = makeApp({ user: admin, isAdmin: true, groups: [group()], createPricingGroup });
    const dialog = await openCreate();
    fireEvent.click(within(dialog).getByRole("button", { name: /^Cancel$/ }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(createPricingGroup).not.toHaveBeenCalled();
  });
});

describe("PricingGroupsWorkspace — pricing-source control", () => {
  it("reflects the group's current pricing-source mode on open", async () => {
    state.app = makeApp({
      user: admin,
      isAdmin: true,
      groups: [group({ id: "GROUP-PS", name: "GROUP-PS", pricingSourceMode: 2 })],
    });
    render(<PricingGroupsWorkspace />);
    fireEvent.click(await screen.findByRole("button", { name: /GROUP-PS/ }));
    const dialog = await screen.findByRole("dialog");

    const select = within(dialog).getByRole("combobox", { name: /Pricing source/ });
    expect((select as HTMLSelectElement).value).toBe("2");
    // Non-skew mode ⇒ no book-skew weight control.
    expect(within(dialog).queryByRole("slider")).toBeNull();
  });

  it("reveals the book-skew weight control ONLY for mode 3", async () => {
    state.app = makeApp({ user: admin, isAdmin: true, groups: [group()] });
    render(<PricingGroupsWorkspace />);
    fireEvent.click(await screen.findByRole("button", { name: /New pricing group/i }));
    const dialog = await screen.findByRole("dialog");

    const select = within(dialog).getByRole("combobox", { name: /Pricing source/ });
    expect(within(dialog).queryByRole("slider")).toBeNull();

    fireEvent.change(select, { target: { value: "3" } });
    expect(within(dialog).getByRole("slider", { name: /Book skew weight/ })).toBeInTheDocument();

    // Switching back hides it again.
    fireEvent.change(select, { target: { value: "1" } });
    expect(within(dialog).queryByRole("slider")).toBeNull();
  });

  it("persists pricing_source_mode on the write, omitting book_skew_weight off mode 3", async () => {
    const createPricingGroup = vi.fn(async (g: PricingGroup) => ({ ...g, id: "GROUP-NEW" }));
    state.app = makeApp({ user: admin, isAdmin: true, groups: [], createPricingGroup });
    render(<PricingGroupsWorkspace />);
    fireEvent.click(await screen.findByRole("button", { name: /New pricing group/i }));
    const dialog = await screen.findByRole("dialog");

    fireEvent.change(within(dialog).getByLabelText(/^Name/), { target: { value: "GROUP-NEW" } });
    fireEvent.change(within(dialog).getByRole("combobox", { name: /Pricing source/ }), {
      target: { value: "1" },
    });

    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: /Create group/i }));
    });

    await waitFor(() => expect(createPricingGroup).toHaveBeenCalledTimes(1));
    const saved = createPricingGroup.mock.calls[0]![0] as PricingGroup;
    expect(saved.pricingSourceMode).toBe(1);
    const wire = pricingGroupSpecToWire(saved);
    expect(wire["pricing_source_mode"]).toBe(1);
    expect("book_skew_weight" in wire).toBe(false);
  });

  it("carries book_skew_weight on the write for mode 3 once the slider is moved", async () => {
    const createPricingGroup = vi.fn(async (g: PricingGroup) => ({ ...g, id: "GROUP-SKEW" }));
    state.app = makeApp({ user: admin, isAdmin: true, groups: [], createPricingGroup });
    render(<PricingGroupsWorkspace />);
    fireEvent.click(await screen.findByRole("button", { name: /New pricing group/i }));
    const dialog = await screen.findByRole("dialog");

    fireEvent.change(within(dialog).getByLabelText(/^Name/), { target: { value: "GROUP-SKEW" } });
    fireEvent.change(within(dialog).getByRole("combobox", { name: /Pricing source/ }), {
      target: { value: "3" },
    });
    fireEvent.change(within(dialog).getByRole("slider", { name: /Book skew weight/ }), {
      target: { value: "0.25" },
    });

    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: /Create group/i }));
    });

    await waitFor(() => expect(createPricingGroup).toHaveBeenCalledTimes(1));
    const wire = pricingGroupSpecToWire(createPricingGroup.mock.calls[0]![0] as PricingGroup);
    expect(wire["pricing_source_mode"]).toBe(3);
    expect(wire["book_skew_weight"]).toBe(0.25);
  });
});

describe("PricingGroupsWorkspace — save closes the modal", () => {
  it("creates through the transport then closes the modal", async () => {
    const createPricingGroup = vi.fn(async (g: PricingGroup) => ({ ...g, id: "GROUP-NEW" }));
    state.app = makeApp({ user: admin, isAdmin: true, groups: [], createPricingGroup });
    render(<PricingGroupsWorkspace />);

    fireEvent.click(await screen.findByRole("button", { name: /New pricing group/i }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText(/^Name/), {
      target: { value: "GROUP-NEW" },
    });

    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: /Create group/i }));
    });

    await waitFor(() => expect(createPricingGroup).toHaveBeenCalledTimes(1));
    expect(createPricingGroup.mock.calls[0]![0].name).toBe("GROUP-NEW");
    // Persisted ⇒ the modal closes.
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});
