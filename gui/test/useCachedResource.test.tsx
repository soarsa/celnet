/**
 * useCachedResource — the stale-while-revalidate cache that fixes the "tables clear
 * and reload on every tab switch" defect. These tests exercise the hook's public
 * behaviour through `renderHook`: cache HIT survives unmount without a refetch,
 * stale lines revalidate in the BACKGROUND without clearing data, concurrent
 * consumers dedupe one in-flight promise, a fetch error keeps the last good data,
 * and `primeCachedResource` seeds a line imperatively.
 *
 * Each test uses a UNIQUE key so the module-level store (which intentionally
 * survives unmount) does not leak between tests.
 */
import { describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import {
  primeCachedResource,
  useCachedResource,
} from "../src/hooks/useCachedResource";

/** A deferred promise we resolve by hand, to control fetch timing precisely. */
function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void } {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

describe("useCachedResource", () => {
  it("cache MISS fetches and populates; a HIT on remount returns data instantly without refetching", async () => {
    const key = `k-hit-${Math.random()}`;
    const fetcher = vi.fn(() => Promise.resolve([1, 2, 3]));

    const first = renderHook(() => useCachedResource(key, fetcher));
    // First render: a miss ⇒ loading, no data.
    expect(first.result.current.isLoading).toBe(true);
    expect(first.result.current.data).toBeUndefined();

    await waitFor(() => expect(first.result.current.data).toEqual([1, 2, 3]));
    expect(first.result.current.isLoading).toBe(false);
    expect(fetcher).toHaveBeenCalledTimes(1);

    first.unmount();

    // Remount within the ttl: the cached data is present on the FIRST render (no
    // blank flash) and the fetcher is NOT called again.
    const second = renderHook(() => useCachedResource(key, fetcher));
    expect(second.result.current.data).toEqual([1, 2, 3]);
    expect(second.result.current.isLoading).toBe(false);
    // Give any (unexpected) effect a tick to fire.
    await act(async () => {
      await Promise.resolve();
    });
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it("revalidates a STALE line in the background, keeping the old data visible until the new lands", async () => {
    const key = `k-stale-${Math.random()}`;
    let n = 0;
    const fetcher = vi.fn(() => Promise.resolve(`v${(n += 1)}`));

    const first = renderHook(() => useCachedResource(key, fetcher, { ttlMs: 0 }));
    await waitFor(() => expect(first.result.current.data).toBe("v1"));
    first.unmount();

    // ttlMs:0 ⇒ the line is immediately stale, so remount revalidates in the
    // background: the old data stays on screen while isValidating flips true.
    const second = renderHook(() => useCachedResource(key, fetcher, { ttlMs: 0 }));
    expect(second.result.current.data).toBe("v1"); // old data kept, no blank
    expect(second.result.current.isLoading).toBe(false); // never a first-load skeleton

    await waitFor(() => expect(second.result.current.data).toBe("v2"));
    expect(fetcher).toHaveBeenCalledTimes(2);
  });

  it("dedupes concurrent consumers of a key onto ONE in-flight fetch", async () => {
    const key = `k-dedupe-${Math.random()}`;
    const d = deferred<number[]>();
    const fetcher = vi.fn(() => d.promise);

    const a = renderHook(() => useCachedResource(key, fetcher));
    const b = renderHook(() => useCachedResource(key, fetcher));

    // Both mounted, both loading — but only one fetch was issued.
    expect(a.result.current.isLoading).toBe(true);
    expect(b.result.current.isLoading).toBe(true);
    expect(fetcher).toHaveBeenCalledTimes(1);

    await act(async () => {
      d.resolve([9]);
      await d.promise;
    });

    await waitFor(() => {
      expect(a.result.current.data).toEqual([9]);
      expect(b.result.current.data).toEqual([9]);
    });
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it("keeps the last good data and exposes the error when a revalidation fails", async () => {
    const key = `k-err-${Math.random()}`;
    const ok = vi.fn(() => Promise.resolve(["good"]));
    const { result, rerender } = renderHook(
      ({ f }: { f: () => Promise<string[]> }) => useCachedResource(key, f, { ttlMs: 60_000 }),
      { initialProps: { f: ok } },
    );
    await waitFor(() => expect(result.current.data).toEqual(["good"]));

    // Swap in a failing fetcher and force a background refresh.
    const boom = new Error("network down");
    const fail = vi.fn(() => Promise.reject(boom));
    rerender({ f: fail });
    act(() => result.current.refresh());

    await waitFor(() => expect(result.current.error).toBe(boom));
    // The last good data is still on screen — the table never blanks on error.
    expect(result.current.data).toEqual(["good"]);
    expect(result.current.isLoading).toBe(false);
  });

  it("primeCachedResource seeds a line so a consumer reads it without fetching", async () => {
    const key = `k-prime-${Math.random()}`;
    primeCachedResource(key, ["seeded"]);
    const fetcher = vi.fn(() => Promise.resolve(["fetched"]));

    const { result } = renderHook(() =>
      useCachedResource(key, fetcher, { ttlMs: 60_000 }),
    );
    // The primed value is visible immediately with no loading state.
    expect(result.current.data).toEqual(["seeded"]);
    expect(result.current.isLoading).toBe(false);
    // A primed line still revalidates once in the background (data kept meanwhile),
    // converging on the fetched value — never a blank in between.
    await waitFor(() => expect(fetcher).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(result.current.data).toEqual(["fetched"]));
  });

  it("does not fetch while disabled", async () => {
    const key = `k-disabled-${Math.random()}`;
    const fetcher = vi.fn(() => Promise.resolve([1]));
    const { result } = renderHook(() =>
      useCachedResource(key, fetcher, { enabled: false }),
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(fetcher).not.toHaveBeenCalled();
    expect(result.current.isLoading).toBe(false);
    expect(result.current.data).toBeUndefined();
  });

  it("keys args separately — a different key is a different cache line", async () => {
    const base = `k-args-${Math.random()}`;
    const fa = vi.fn(() => Promise.resolve("A"));
    const fb = vi.fn(() => Promise.resolve("B"));
    const a = renderHook(() => useCachedResource(`${base}|a`, fa));
    const b = renderHook(() => useCachedResource(`${base}|b`, fb));
    await waitFor(() => {
      expect(a.result.current.data).toBe("A");
      expect(b.result.current.data).toBe("B");
    });
  });
});
