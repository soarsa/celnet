/**
 * Shared test helpers.
 *
 * `tsconfig.app.json` enables `noUncheckedIndexedAccess`, so `xs[0]` is typed
 * `T | undefined` — correct for production code, but it makes ordinary test
 * assertions (`expect(users[0].deskIds)`) fail to compile. `expect(x).toBeDefined()`
 * does NOT narrow the binding for the compiler either.
 *
 * These helpers assert-and-return: they narrow the type AND fail with a precise
 * message naming the missing element, instead of surfacing later as a property
 * access on `undefined`. Prefer them over a non-null assertion (`xs[0]!`), which
 * silences the compiler without improving the failure.
 */

/** The element at `index`, asserted present. */
export function at<T>(xs: ArrayLike<T> | readonly T[], index: number): T {
  const v = (xs as readonly T[])[index];
  if (v === undefined) {
    throw new Error(
      `expected an element at index ${index}, but the collection has ${xs.length}`,
    );
  }
  return v;
}

/** The first element, asserted present. */
export function first<T>(xs: ArrayLike<T> | readonly T[]): T {
  return at(xs, 0);
}

/** The value for `key`, asserted present. */
export function get<K, V>(map: ReadonlyMap<K, V>, key: K): V {
  const v = map.get(key);
  if (v === undefined) {
    throw new Error(`expected the map to hold a value for ${String(key)}`);
  }
  return v;
}
