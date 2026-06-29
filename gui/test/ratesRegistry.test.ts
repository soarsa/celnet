import { describe, expect, it } from "vitest";

import { createMockTransport } from "../src/data/mockSource";

/**
 * Behavioural tests for the legal-entity / netting-book registry transport
 * surface. The offline `MockTransport` is the FULL real implementation (no stub)
 * seeded to MIRROR the server's default registry, so the lifecycle invariants the
 * live `AuthService` edge must satisfy — list is open, create auto-assigns the
 * lowest free key, update keeps the key immutable, deleting an entity is refused
 * while a book references it — are asserted here against the same contract the
 * GUI calls.
 */

describe("entity / book registry — offline lifecycle", () => {
  it("seeds the same entities + books as the server default registry", async () => {
    const t = createMockTransport();
    const entities = await t.listEntities();
    expect(entities).toEqual([
      { key: 1, name: "Celnet Global Markets", code: "CGM" },
      { key: 2, name: "Celnet Securities", code: "CSEC" },
    ]);
    const books = await t.listBooks();
    expect(books.map((b) => b.name)).toEqual([
      "Rates Trading",
      "Rates Relative Value",
      "Government Bonds",
      "Swaps",
    ]);
    // Every seeded book references an existing entity.
    for (const book of books) {
      expect(entities.some((e) => e.key === book.entityKey)).toBe(true);
    }
  });

  it("creates an entity auto-assigning the lowest free key", async () => {
    const t = createMockTransport();
    const created = await t.createEntity({ name: "ACME Capital", code: "ACME" });
    expect(created).toEqual({ key: 3, name: "ACME Capital", code: "ACME" });
    const entities = await t.listEntities();
    expect(entities.some((e) => e.key === 3 && e.name === "ACME Capital")).toBe(true);
  });

  it("rejects a duplicate entity name and code", async () => {
    const t = createMockTransport();
    await expect(t.createEntity({ name: "Celnet Global Markets", code: "X" })).rejects.toThrow();
    await expect(t.createEntity({ name: "Y", code: "CGM" })).rejects.toThrow();
  });

  it("updates an entity in place keeping the key immutable", async () => {
    const t = createMockTransport();
    const updated = await t.updateEntity(2, { name: "Celnet Securities Ltd", code: "CSL" });
    expect(updated).toEqual({ key: 2, name: "Celnet Securities Ltd", code: "CSL" });
    const entities = await t.listEntities();
    expect(entities.find((e) => e.key === 2)?.name).toBe("Celnet Securities Ltd");
  });

  it("refuses to delete an entity while a book still references it", async () => {
    const t = createMockTransport();
    // Entity 1 owns seeded books — deletion must be refused (server parity).
    await expect(t.deleteEntity(1)).rejects.toThrow();
    expect((await t.listEntities()).some((e) => e.key === 1)).toBe(true);
  });

  it("deletes an entity once its books are removed", async () => {
    const t = createMockTransport();
    const entity = await t.createEntity({ name: "ACME Capital", code: "ACME" });
    const book = await t.createBook({ name: "ACME Rates", entityKey: entity.key });
    await expect(t.deleteEntity(entity.key)).rejects.toThrow();
    expect(await t.deleteBook(book.key)).toBe(true);
    expect(await t.deleteEntity(entity.key)).toBe(true);
    expect((await t.listEntities()).some((e) => e.key === entity.key)).toBe(false);
  });

  it("creates a book under an entity auto-assigning the lowest free key", async () => {
    const t = createMockTransport();
    const created = await t.createBook({ name: "Rates Trading Desk 2", entityKey: 1 });
    expect(created).toEqual({ key: 5, name: "Rates Trading Desk 2", entityKey: 1 });
  });

  it("rejects a book under an unknown entity", async () => {
    const t = createMockTransport();
    await expect(t.createBook({ name: "Orphan", entityKey: 999 })).rejects.toThrow();
  });

  it("re-homes a book to a different entity on update (key immutable)", async () => {
    const t = createMockTransport();
    const updated = await t.updateBook(1, { name: "Rates Trading", entityKey: 2 });
    expect(updated).toEqual({ key: 1, name: "Rates Trading", entityKey: 2 });
  });
});
