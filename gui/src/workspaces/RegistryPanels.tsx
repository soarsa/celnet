/**
 * RegistryPanels — the admin-managed legal-entity + netting-book registry panels
 * for the Admin workspace. Each panel mirrors the Desks panel (list + create /
 * edit / delete), admin-gated by the parent. The registry names the `(entity,
 * book)` `uint32` partition keys a `RatesPosition` books into; the rates booking
 * form resolves a NAMED selection to those keys (the position wire is unchanged).
 *
 * The Books form lets the admin pick the owning entity from a `<select>` of
 * entities and resolves each book's `entityKey` to its entity NAME for display —
 * a raw key is never shown. Mutations go through the parent's `run` wrapper (which
 * surfaces failures), clearing the form only after a successful await (mirroring
 * the Desks form). A book cannot be created without at least one entity.
 */

import { useState } from "react";

import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import type { BookDesc, BookInput, EntityDesc, EntityInput } from "../data/contract";
import styles from "./AdminWorkspace.module.css";

/** Resolve an `entityKey` to its display name, falling back to `#<key>`. */
export function entityNameOf(entities: readonly EntityDesc[], key: number): string {
  const entity = entities.find((e) => e.key === key);
  return entity ? entity.name : `#${key}`;
}

interface EntitiesPanelProps {
  entities: EntityDesc[];
  books: BookDesc[];
  onCreate: (input: EntityInput) => Promise<unknown>;
  onUpdate: (key: number, input: EntityInput) => Promise<unknown>;
  onDelete: (key: number) => Promise<unknown>;
  run: (action: () => Promise<unknown>) => Promise<void>;
}

/** The editable entity form model; `key === null` ⇒ create mode. */
interface EntityForm {
  key: number | null;
  name: string;
  code: string;
}

const EMPTY_ENTITY_FORM: EntityForm = { key: null, name: "", code: "" };

export function EntitiesPanel({
  entities,
  books,
  onCreate,
  onUpdate,
  onDelete,
  run,
}: EntitiesPanelProps): React.ReactElement {
  const [form, setForm] = useState<EntityForm>(EMPTY_ENTITY_FORM);

  const name = form.name.trim();
  const code = form.code.trim();
  const isEditing = form.key !== null;
  const canSubmit = name.length > 0 && code.length > 0;
  const bookCount = (entityKey: number): number =>
    books.filter((b) => b.entityKey === entityKey).length;

  const submit = (e: React.FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (!canSubmit) return;
    void run(async () => {
      if (form.key === null) {
        await onCreate({ name, code });
      } else {
        await onUpdate(form.key, { name, code });
      }
      setForm(EMPTY_ENTITY_FORM);
    });
  };

  return (
    <Panel title="Legal entities" glyph="◷">
      {entities.length === 0 ? (
        <p className={styles.empty}>
          No entities yet. An entity is the legal account a rates position books into.
        </p>
      ) : (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>Key</th>
              <th>Name</th>
              <th>Code</th>
              <th>Books</th>
              <th className={styles.actionsCol}>Actions</th>
            </tr>
          </thead>
          <tbody>
            {entities.map((entity) => (
              <tr key={entity.key}>
                <td className={styles.mono}>{entity.key}</td>
                <td className={styles.nameCell}>{entity.name}</td>
                <td className={styles.mono}>{entity.code}</td>
                <td className={styles.mono}>{bookCount(entity.key)}</td>
                <td className={styles.actionsCol}>
                  <div className={styles.rowActions}>
                    <Button
                      variant="secondary"
                      onClick={() =>
                        setForm({ key: entity.key, name: entity.name, code: entity.code })
                      }
                    >
                      Edit
                    </Button>
                    <Button variant="ghost" onClick={() => void run(() => onDelete(entity.key))}>
                      Delete
                    </Button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <form className={styles.deskForm} onSubmit={submit}>
        <input
          className={styles.deskInput}
          type="text"
          value={form.name}
          onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
          placeholder="Entity name, e.g. ACME Capital"
          aria-label="entity name"
        />
        <input
          className={styles.deskInput}
          type="text"
          value={form.code}
          onChange={(e) => setForm((f) => ({ ...f, code: e.target.value }))}
          placeholder="Code, e.g. ACME"
          aria-label="entity code"
        />
        <Button type="submit" variant="secondary" disabled={!canSubmit}>
          {isEditing ? "Save entity" : "Add entity"}
        </Button>
        {isEditing && (
          <Button type="button" variant="ghost" onClick={() => setForm(EMPTY_ENTITY_FORM)}>
            Cancel
          </Button>
        )}
      </form>
    </Panel>
  );
}

interface BooksPanelProps {
  entities: EntityDesc[];
  books: BookDesc[];
  onCreate: (input: BookInput) => Promise<unknown>;
  onUpdate: (key: number, input: BookInput) => Promise<unknown>;
  onDelete: (key: number) => Promise<unknown>;
  run: (action: () => Promise<unknown>) => Promise<void>;
}

/** The editable book form model; `key === null` ⇒ create mode. */
interface BookForm {
  key: number | null;
  name: string;
  entityKey: number | null;
}

export function BooksPanel({
  entities,
  books,
  onCreate,
  onUpdate,
  onDelete,
  run,
}: BooksPanelProps): React.ReactElement {
  const emptyForm: BookForm = { key: null, name: "", entityKey: null };
  const [form, setForm] = useState<BookForm>(emptyForm);

  const hasEntities = entities.length > 0;
  const name = form.name.trim();
  const isEditing = form.key !== null;
  const canSubmit = name.length > 0 && form.entityKey !== null;

  const submit = (e: React.FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (name.length === 0 || form.entityKey === null) return;
    const entityKey = form.entityKey;
    void run(async () => {
      if (form.key === null) {
        await onCreate({ name, entityKey });
      } else {
        await onUpdate(form.key, { name, entityKey });
      }
      setForm(emptyForm);
    });
  };

  return (
    <Panel title="Netting books" glyph="▤">
      {books.length === 0 ? (
        <p className={styles.empty}>
          No books yet. A book is a netting cell under an entity that positions book into.
        </p>
      ) : (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>Key</th>
              <th>Name</th>
              <th>Entity</th>
              <th className={styles.actionsCol}>Actions</th>
            </tr>
          </thead>
          <tbody>
            {books.map((book) => (
              <tr key={book.key}>
                <td className={styles.mono}>{book.key}</td>
                <td className={styles.nameCell}>{book.name}</td>
                <td>{entityNameOf(entities, book.entityKey)}</td>
                <td className={styles.actionsCol}>
                  <div className={styles.rowActions}>
                    <Button
                      variant="secondary"
                      onClick={() =>
                        setForm({ key: book.key, name: book.name, entityKey: book.entityKey })
                      }
                    >
                      Edit
                    </Button>
                    <Button variant="ghost" onClick={() => void run(() => onDelete(book.key))}>
                      Delete
                    </Button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {!hasEntities ? (
        <p className={styles.empty}>Add a legal entity first — a book must belong to one.</p>
      ) : (
        <form className={styles.deskForm} onSubmit={submit}>
          <input
            className={styles.deskInput}
            type="text"
            value={form.name}
            onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
            placeholder="Book name, e.g. Rates Trading"
            aria-label="book name"
          />
          <select
            className={styles.deskInput}
            value={form.entityKey ?? ""}
            onChange={(e) =>
              setForm((f) => ({
                ...f,
                entityKey: e.target.value === "" ? null : Number(e.target.value),
              }))
            }
            aria-label="owning entity"
          >
            <option value="">Owning entity…</option>
            {entities.map((entity) => (
              <option key={entity.key} value={entity.key}>
                {entity.name}
              </option>
            ))}
          </select>
          <Button type="submit" variant="secondary" disabled={!canSubmit}>
            {isEditing ? "Save book" : "Add book"}
          </Button>
          {isEditing && (
            <Button type="button" variant="ghost" onClick={() => setForm(emptyForm)}>
              Cancel
            </Button>
          )}
        </form>
      )}
    </Panel>
  );
}
