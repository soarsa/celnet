/**
 * UserDialog — the create / edit / reset-password modal for the Admin workspace.
 *
 * One modal, three modes over the `AuthService` admin surface:
 *  - `create`: email + display name + role + desk + initial password → `CreateUser`.
 *  - `edit`: display name + role + desk + disabled (email read-only) → `UpdateUser`.
 *  - `reset`: a new password for the named user → `ResetPassword`.
 *
 * Passwords are validated to the server's 12-character minimum client-side so the
 * operator gets immediate feedback; the server re-checks authoritatively. On
 * success the dialog closes; a server rejection (duplicate email, last-admin
 * lockout, weak password) is shown inline for correction.
 *
 * Accessibility: `role="dialog"` + `aria-modal`, labelled title, Esc to close,
 * initial focus on the first editable control.
 */

import { useEffect, useId, useRef, useState } from "react";

import type {
  CreateUserInput,
  DeskDesc,
  UpdateUserInput,
  UserDesc,
  UserRole,
} from "../data/contract";
import { Button } from "./Button";
import styles from "./UserDialog.module.css";

/** The server's minimum password length (mirrors `MIN_PASSWORD_LEN`). */
const MIN_PASSWORD_LEN = 12;

export type UserDialogMode = "create" | "edit" | "reset";

export interface UserDialogProps {
  open: boolean;
  mode: UserDialogMode;
  /** The target user (required for `edit`/`reset`; ignored/absent for `create`). */
  user?: UserDesc | undefined;
  /** The desks available for assignment. */
  desks: readonly DeskDesc[];
  onClose: () => void;
  onCreate: (input: CreateUserInput) => Promise<unknown>;
  onUpdate: (id: string, input: UpdateUserInput) => Promise<unknown>;
  onReset: (id: string, newPassword: string) => Promise<unknown>;
}

const TITLES: Record<UserDialogMode, string> = {
  create: "New user",
  edit: "Edit user",
  reset: "Reset password",
};

export function UserDialog({
  open,
  mode,
  user,
  desks,
  onClose,
  onCreate,
  onUpdate,
  onReset,
}: UserDialogProps): React.ReactElement | null {
  const titleId = useId();
  const firstRef = useRef<HTMLInputElement | null>(null);

  const [email, setEmail] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [role, setRole] = useState<UserRole>("TRADER");
  const [deskId, setDeskId] = useState("");
  const [password, setPassword] = useState("");
  const [disabled, setDisabled] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Seed the form from the target (edit/reset) or to clean defaults (create)
  // whenever the dialog (re)opens.
  useEffect(() => {
    if (!open) return;
    setEmail(user?.email ?? "");
    setDisplayName(user?.displayName ?? "");
    setRole(user?.role ?? "TRADER");
    setDeskId(user?.deskId ?? "");
    setPassword("");
    setDisabled(user?.disabled ?? false);
    setSubmitting(false);
    setError(null);
  }, [open, user]);

  useEffect(() => {
    if (open) firstRef.current?.focus();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  const needsPassword = mode === "create" || mode === "reset";
  const passwordOk = !needsPassword || password.length >= MIN_PASSWORD_LEN;
  const emailOk = mode !== "create" || email.trim().length > 0;
  const canSubmit = emailOk && passwordOk && !submitting;

  const submit = (): void => {
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    const desk = deskId.trim();
    const run = async (): Promise<void> => {
      if (mode === "create") {
        const input: CreateUserInput = {
          email: email.trim(),
          displayName: displayName.trim(),
          role,
          password,
        };
        if (desk.length > 0) input.deskId = desk;
        await onCreate(input);
      } else if (mode === "edit" && user) {
        const input: UpdateUserInput = {
          displayName: displayName.trim(),
          role,
          disabled,
        };
        if (desk.length > 0) input.deskId = desk;
        await onUpdate(user.id, input);
      } else if (mode === "reset" && user) {
        await onReset(user.id, password);
      }
    };
    void run()
      .then(() => onClose())
      .catch((e: unknown) => {
        setError(e instanceof Error ? e.message : "action failed");
        setSubmitting(false);
      });
  };

  const handleSubmit = (event: React.FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    submit();
  };

  return (
    <div
      className={styles.scrim}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className={styles.panel} role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <div className={styles.head}>
          <h2 id={titleId} className={styles.title}>
            {TITLES[mode]}
          </h2>
        </div>

        <form className={styles.form} onSubmit={handleSubmit}>
          {mode === "create" ? (
            <label className={styles.field}>
              <span className={styles.fieldLabel}>Email</span>
              <input
                ref={firstRef}
                className={styles.input}
                type="email"
                autoComplete="off"
                placeholder="trader@celnet.com"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
            </label>
          ) : (
            <div className={styles.field}>
              <span className={styles.fieldLabel}>Email</span>
              <span className={styles.readonly}>{user?.email}</span>
            </div>
          )}

          {mode !== "reset" && (
            <>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>Display name</span>
                <input
                  ref={mode === "edit" ? firstRef : undefined}
                  className={styles.input}
                  type="text"
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                  placeholder="Jane Trader"
                />
              </label>

              <label className={styles.field}>
                <span className={styles.fieldLabel}>Role</span>
                <select
                  className={styles.select}
                  value={role}
                  onChange={(e) => setRole(e.target.value === "ADMIN" ? "ADMIN" : "TRADER")}
                >
                  <option value="TRADER">Trader</option>
                  <option value="ADMIN">Administrator</option>
                </select>
              </label>

              <label className={styles.field}>
                <span className={styles.fieldLabel}>Desk</span>
                <select
                  className={styles.select}
                  value={deskId}
                  onChange={(e) => setDeskId(e.target.value)}
                >
                  <option value="">— unassigned</option>
                  {desks.map((d) => (
                    <option key={d.id} value={d.id}>
                      {d.name} ({d.id})
                    </option>
                  ))}
                </select>
              </label>
            </>
          )}

          {needsPassword && (
            <label className={styles.field}>
              <span className={styles.fieldLabel}>
                {mode === "reset" ? "New password" : "Initial password"}
              </span>
              <input
                ref={mode === "reset" ? firstRef : undefined}
                className={styles.input}
                type="password"
                autoComplete="new-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder="at least 12 characters"
                aria-invalid={password.length > 0 && !passwordOk}
              />
              <p className={styles.hint}>Minimum {MIN_PASSWORD_LEN} characters.</p>
            </label>
          )}

          {mode === "edit" && (
            <label className={styles.checkRow}>
              <input
                type="checkbox"
                checked={disabled}
                onChange={(e) => setDisabled(e.target.checked)}
              />
              <span>Disabled (account retained but cannot sign in)</span>
            </label>
          )}

          {error && (
            <p className={styles.error} role="alert">
              {error}
            </p>
          )}

          <div className={styles.foot}>
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={!canSubmit}>
              {submitting ? "Saving…" : mode === "create" ? "Create user" : "Save"}
            </Button>
          </div>
        </form>
      </div>
    </div>
  );
}
