/**
 * W4 exception notifications — OFFLINE mock-transport e2e (the `?mock` posture, no
 * cargo/edge). Proves the server exception-contract POPUP gate end-to-end through
 * the real {@link NotificationCenter} + {@link useNotificationStore} +
 * {@link useDesktopNotifications}: a few seconds after the centre subscribes, the
 * mock source emits two exception-contract sample events —
 *   (a) a QUIET auto-priced event (`alertWorthy:false`) that must land ONLY in the
 *       notification-centre list (NO toast, NO desktop growl), and
 *   (b) an alert-worthy `MANUAL_INTERVENTION_REQUIRED` event (`alertWorthy:true`,
 *       reason `UNCONFIGURED_TENOR`) that must POP an in-app growl toast carrying the
 *       trader-facing reason, and attempt the desktop ("growl") escalation.
 *
 * The browser Notification API is FAKED before app load (a granted-permission
 * constructor recording every `new Notification(...)`), and the tab is made to
 * appear "away" (`document.hasFocus() === false`) so the desktop-growl path — which
 * only fires when the tab is hidden/unfocused — is exercised deterministically. The
 * in-app growl TOAST fires regardless of tab visibility, so it is the primary,
 * reliable assertion.
 *
 * Auth note: the `AuthGate` gates the Shell even under `?mock`, but the MOCK
 * `AuthService` seeds `admin@celnet.com` / `password` and authenticates OFFLINE, so
 * we sign in through the mock login (NOT `openLive`/`signIn`, which need the server).
 * The notification stream is global, so the default FX surface is fine.
 */
import { test, expect, type Page } from "@playwright/test";

/** Sign in through the OFFLINE mock login gate (seeded admin), then settle. */
async function signInMock(page: Page): Promise<void> {
  const heading = page.getByRole("heading", { name: "Sign in to Celnet" });
  await expect(heading).toBeVisible();
  await page.getByLabel("Email").fill("admin@celnet.com");
  await page.getByLabel("Password").fill("password");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(heading).toBeHidden();
}

test("exception popup gate: alert-worthy manual-intervention pops (toast + desktop) with its reason; quiet auto-priced stays silent in the centre", async ({
  page,
}) => {
  // Fake the browser Notification API BEFORE the app loads: a granted-permission
  // constructor that records each `new Notification(...)` to a global array, plus a
  // tab-away override so the desktop-growl escalation path is deterministically
  // reachable (the in-app toast fires regardless).
  await page.addInitScript(() => {
    const w = window as unknown as {
      __celnetNotifications: { title: string; body: string }[];
      Notification: unknown;
    };
    w.__celnetNotifications = [];
    class FakeNotification {
      title: string;
      body: string;
      onclick: (() => void) | null = null;
      constructor(title: string, options?: { body?: string }) {
        this.title = title;
        this.body = options?.body ?? "";
        w.__celnetNotifications.push({ title, body: this.body });
      }
      close(): void {}
      static permission = "granted";
      static requestPermission(cb?: (p: string) => void): Promise<string> {
        cb?.("granted");
        return Promise.resolve("granted");
      }
    }
    w.Notification = FakeNotification;
    // Make the tab appear hidden/unfocused so `useDesktopNotifications` escalates.
    document.hasFocus = () => false;
  });

  await page.goto("/?mock");
  await signInMock(page);

  // (b) The alert-worthy MANUAL_INTERVENTION_REQUIRED event POPS an in-app growl
  // toast (the toasts region is `role="status"`), headlined "Manual pricing needed"
  // and carrying the trader-facing reason ("unconfigured tenor"). The centre list is
  // closed on first paint, so the ONLY button bearing this text is the toast.
  const toastRegion = page.getByRole("status");
  await expect(
    toastRegion.getByRole("button", { name: /Manual pricing needed/ }),
  ).toBeVisible({ timeout: 15_000 });
  await expect(toastRegion.getByText(/unconfigured tenor/i)).toBeVisible();

  // (a) The QUIET auto-priced event (`alertWorthy:false`) NEVER toasts — no toast in
  // the status region mentions it. (It fired at 1.5s, before the manual alert at 3s,
  // so by the time the manual toast is up the quiet event has provably not popped.)
  await expect(
    toastRegion.getByRole("button", { name: /Auto-priced|Aster Global/ }),
  ).toHaveCount(0);

  // The quiet event DID land in the notification-centre list. Open the bell and
  // assert the quiet item is present (its notional compacted "25mm" → "25m").
  await page.getByRole("button", { name: /^Notifications,/ }).click();
  const dropdown = page.getByRole("group", { name: "Recent notifications" });
  await expect(dropdown).toBeVisible();
  await expect(dropdown.getByText(/Auto-priced RFQ from Aster Global/)).toBeVisible();

  // Desktop ("growl") path: the alert-worthy manual-intervention event attempted a
  // native OS notification carrying the reason in its body; the quiet event did NOT.
  const notes = await page.evaluate(
    () =>
      (window as unknown as { __celnetNotifications?: { title: string; body: string }[] })
        .__celnetNotifications ?? [],
  );
  expect(
    notes.some(
      (n) => n.title === "Manual pricing needed" && /unconfigured tenor/i.test(n.body),
    ),
  ).toBe(true);
  expect(
    notes.some((n) => /Auto-priced|Aster/i.test(n.title) || /Auto-priced|Aster/i.test(n.body)),
  ).toBe(false);
});
