/**
 * useTableUiState — the per-table UI-state store that keeps a trader's sort / search
 * / lens / scroll where they left it across a tab switch (the workspace unmounting).
 * These tests confirm state survives unmount, `patch` merges immutably, and distinct
 * ids are isolated.
 */
import { describe, expect, it } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useTableUiState } from "../src/hooks/useTableUiState";

interface View {
  sort: string;
  query: string;
}

describe("useTableUiState", () => {
  it("returns the initial state on first mount and merges partial patches immutably", () => {
    const id = `t-init-${Math.random()}`;
    const { result } = renderHook(() =>
      useTableUiState<View>(id, { sort: "gross", query: "" }),
    );
    expect(result.current[0]).toEqual({ sort: "gross", query: "" });

    act(() => result.current[1]({ query: "meridian" }));
    // The untouched field is preserved; only `query` changed.
    expect(result.current[0]).toEqual({ sort: "gross", query: "meridian" });
  });

  it("persists state across unmount and restores it on remount", () => {
    const id = `t-persist-${Math.random()}`;
    const first = renderHook(() =>
      useTableUiState<View>(id, { sort: "gross", query: "" }),
    );
    act(() => first.result.current[1]({ sort: "net", query: "abc" }));
    first.unmount();

    // A fresh mount with the SAME id ignores the initial and restores the last state.
    const second = renderHook(() =>
      useTableUiState<View>(id, { sort: "gross", query: "" }),
    );
    expect(second.result.current[0]).toEqual({ sort: "net", query: "abc" });
  });

  it("isolates distinct table ids", () => {
    const idA = `t-iso-a-${Math.random()}`;
    const idB = `t-iso-b-${Math.random()}`;
    const a = renderHook(() => useTableUiState<View>(idA, { sort: "gross", query: "" }));
    const b = renderHook(() => useTableUiState<View>(idB, { sort: "gross", query: "" }));

    act(() => a.result.current[1]({ query: "only-a" }));
    expect(a.result.current[0].query).toBe("only-a");
    expect(b.result.current[0].query).toBe("");
  });
});
