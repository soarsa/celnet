/**
 * UserDialog — the create/edit/reset modal behind the Admin workspace's user
 * management. These assert the create form genuinely works end-to-end through the
 * component: typing populates the fields and a valid submit calls `onCreate` with
 * the typed values, while an invalid one surfaces a clear inline reason instead of
 * a silently-inert button (the "clicking create does nothing" failure mode).
 */

import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { UserDialog } from "../src/components/UserDialog";

function noop(): Promise<void> {
  return Promise.resolve();
}

describe("UserDialog — create", () => {
  it("submits the typed values when email + a 12-char password are entered", async () => {
    const onCreate = vi.fn().mockResolvedValue({});
    const onClose = vi.fn();
    render(
      <UserDialog
        open
        mode="create"
        desks={[{ id: "g10", name: "G10 Options" }]}
        onClose={onClose}
        onCreate={onCreate}
        onUpdate={noop}
        onReset={noop}
      />,
    );

    fireEvent.change(screen.getByPlaceholderText("trader@celnet.com"), {
      target: { value: "jane@celnet.com" },
    });
    fireEvent.change(screen.getByPlaceholderText("Jane Trader"), {
      target: { value: "Jane" },
    });
    fireEvent.change(screen.getByPlaceholderText("at least 12 characters"), {
      target: { value: "longenoughpw1" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create user" }));

    await waitFor(() => expect(onCreate).toHaveBeenCalledTimes(1));
    expect(onCreate).toHaveBeenCalledWith(
      expect.objectContaining({
        email: "jane@celnet.com",
        displayName: "Jane",
        role: "TRADER",
        password: "longenoughpw1",
        // No desks ticked ⇒ a deskless user (empty set, not all-desks).
        deskIds: [],
        allDesks: false,
      }),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("submits the ticked desks as a multi-desk set (allDesks:false)", async () => {
    const onCreate = vi.fn().mockResolvedValue({});
    render(
      <UserDialog
        open
        mode="create"
        desks={[
          { id: "g10", name: "G10 Options" },
          { id: "em", name: "EM Rates" },
        ]}
        onClose={vi.fn()}
        onCreate={onCreate}
        onUpdate={noop}
        onReset={noop}
      />,
    );

    fireEvent.change(screen.getByPlaceholderText("trader@celnet.com"), {
      target: { value: "jane@celnet.com" },
    });
    fireEvent.change(screen.getByPlaceholderText("at least 12 characters"), {
      target: { value: "longenoughpw1" },
    });
    fireEvent.click(screen.getByRole("checkbox", { name: "G10 Options" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "EM Rates" }));
    fireEvent.click(screen.getByRole("button", { name: "Create user" }));

    await waitFor(() => expect(onCreate).toHaveBeenCalledTimes(1));
    expect(onCreate).toHaveBeenCalledWith(
      expect.objectContaining({ deskIds: ["g10", "em"], allDesks: false }),
    );
  });

  it("submits allDesks:true with an empty set when All desks is toggled on", async () => {
    const onCreate = vi.fn().mockResolvedValue({});
    render(
      <UserDialog
        open
        mode="create"
        desks={[{ id: "g10", name: "G10 Options" }]}
        onClose={vi.fn()}
        onCreate={onCreate}
        onUpdate={noop}
        onReset={noop}
      />,
    );

    fireEvent.change(screen.getByPlaceholderText("trader@celnet.com"), {
      target: { value: "chief@celnet.com" },
    });
    fireEvent.change(screen.getByPlaceholderText("at least 12 characters"), {
      target: { value: "longenoughpw1" },
    });
    // Tick a desk first, then All desks — All supersedes the explicit set.
    fireEvent.click(screen.getByRole("checkbox", { name: "G10 Options" }));
    fireEvent.click(screen.getByRole("checkbox", { name: /All desks/ }));
    // The per-desk list is hidden once All desks is on.
    expect(screen.queryByRole("checkbox", { name: "G10 Options" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Create user" }));

    await waitFor(() => expect(onCreate).toHaveBeenCalledTimes(1));
    expect(onCreate).toHaveBeenCalledWith(
      expect.objectContaining({ deskIds: [], allDesks: true }),
    );
  });

  it("shows an inline reason (not a dead button) when the password is too short", () => {
    const onCreate = vi.fn().mockResolvedValue({});
    render(
      <UserDialog
        open
        mode="create"
        desks={[]}
        onClose={vi.fn()}
        onCreate={onCreate}
        onUpdate={noop}
        onReset={noop}
      />,
    );

    fireEvent.change(screen.getByPlaceholderText("trader@celnet.com"), {
      target: { value: "jane@celnet.com" },
    });
    fireEvent.change(screen.getByPlaceholderText("at least 12 characters"), {
      target: { value: "short" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create user" }));

    expect(onCreate).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent ?? "").toMatch(/at least 12 characters/);
  });

  it("requires an email", () => {
    const onCreate = vi.fn().mockResolvedValue({});
    render(
      <UserDialog
        open
        mode="create"
        desks={[]}
        onClose={vi.fn()}
        onCreate={onCreate}
        onUpdate={noop}
        onReset={noop}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Create user" }));
    expect(onCreate).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent ?? "").toMatch(/email is required/i);
  });
});
