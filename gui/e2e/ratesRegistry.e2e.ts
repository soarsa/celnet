/**
 * Live end-to-end (against the booted demo edge under CELNET_ACCESS_MODE=enforce):
 * the admin-managed legal-entity / netting-book registry drives a NAMED rates
 * booking, and the book/blotter render NAMES — never the raw uint32 partition keys.
 *
 * Walkthrough: admin signs in → Admin workspace → create a legal entity + a book
 * under it → the Book workspace's "Positions & Booking" lens → confirm Entity &
 * Book are NAMED dropdowns → book a position → confirm the rates book shows the
 * NAMES. (fe-fi-migration #6 retired the FX/FI domain-tab split: every workspace,
 * including the rates book, is a direct button on the single class-parametric
 * rail.) Screenshots at 1440 width capture (a) the named dropdowns and (b) the
 * booked line showing names.
 *
 * The seeded edge already owns the sample registry (Celnet Global Markets /
 * Celnet Securities + the "Rates Trading" book), and book names are globally
 * unique server-side, so this walkthrough uses run-unique names (a timestamp
 * suffix) to prove admin CRUD without colliding with the seed.
 */
import { test, expect, type Page } from "@playwright/test";

import { openLive, expectNoSeriousA11y } from "./helpers";

const ARTIFACTS = "e2e/.artifacts";

/** Open a workspace via its direct rail button (by `title="<label> (…)"` prefix), return its pane. */
async function gotoView(page: Page, label: string) {
  const rail = page.getByRole("complementary", { name: "workspaces" });
  const btn = rail.locator(`button[title^="${label} ("]`);
  // fe-fi-migration re-add: the rail is filtered to the active domain. If the row
  // isn't under the current tab, select whichever domain tab surfaces it first.
  if ((await btn.count()) === 0) {
    const tabs = page.getByRole("tablist", { name: "product domains" }).getByRole("tab");
    for (let i = 0; i < (await tabs.count()); i += 1) {
      await tabs.nth(i).click();
      if ((await btn.count()) > 0) break;
    }
  }
  await btn.click();
  const pane = page.locator('[aria-hidden="false"]:not([inert])').last();
  await expect(pane).toBeVisible();
  return pane;
}

/**
 * Open the rates book. fe-fi-migration #6 collapsed the standalone "Rates Book"
 * rail row into the single class-parametric Book workspace: go to Book, then flip
 * to the "Positions & Booking" lens in the book-view group — the rates booking
 * form (named entity/book dropdowns + the blotter) lives there.
 */
async function gotoRatesBook(page: Page) {
  const pane = await gotoView(page, "Book");
  await pane
    .getByRole("group", { name: "book view" })
    .getByRole("button", { name: "Positions & Booking" })
    .click();
  return pane;
}

test("admin registry → named rates booking renders names, not numbers", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openLive(page);

  const stamp = Date.now().toString().slice(-6);
  const entityName = `ACME Capital ${stamp}`;
  const entityCode = `AC${stamp}`;
  const bookName = `Rates Trading ${stamp}`;

  // --- Admin workspace: create the legal entity + a book under it -------------
  const admin = await gotoView(page, "Admin");

  // Legal entities live in their own section tab now.
  await admin.getByRole("tab", { name: "Legal Entities" }).click();
  await admin.getByLabel("entity name").fill(entityName);
  await admin.getByLabel("entity code").fill(entityCode);
  await admin.getByRole("button", { name: "Add entity" }).click();
  await expect(admin.getByRole("cell", { name: entityName, exact: true })).toBeVisible();

  // Netting books live in the Netting Books section tab.
  await admin.getByRole("tab", { name: "Netting Books" }).click();
  await admin.getByLabel("book name").fill(bookName);
  await admin.getByLabel("owning entity").selectOption({ label: entityName });
  await admin.getByRole("button", { name: "Add book" }).click();
  // The book row resolves the owning entity to its NAME (not the key).
  const bookRow = admin.getByRole("row", { name: new RegExp(bookName.replace(/\s/g, "\\s")) });
  await expect(bookRow).toContainText(entityName);

  await expectNoSeriousA11y(page, "Administration (entity + book registry)");

  // --- Book workspace (Positions & Booking lens): the Book Position form's NAMED dropdowns ---
  const ratesBook = await gotoRatesBook(page);

  // Both are <select> dropdowns: a native <select> has the implicit ARIA role
  // "combobox", so resolving them by that role proves they are NAMED dropdowns
  // (not the former number inputs).
  const entitySelect = ratesBook.getByRole("combobox", { name: "legal entity" });
  const bookSelect = ratesBook.getByRole("combobox", { name: "netting book" });
  await expect(entitySelect).toBeVisible();
  await expect(bookSelect).toBeVisible();

  // Pick the just-created NAMED entity + book.
  await entitySelect.selectOption({ label: entityName });
  await bookSelect.selectOption({ label: bookName });
  await expect(bookSelect.locator("option:checked")).toHaveText(bookName);

  // (a) screenshot: the form's named dropdowns.
  await page.screenshot({ path: `${ARTIFACTS}/rates-form-named-dropdowns.png` });

  // Book the position (Notional/Tenor/Fixed/Side keep their seeded defaults).
  await ratesBook.getByRole("button", { name: "Book position" }).click();

  // The booked line appears in the rates book table showing the NAMES.
  const table = ratesBook.getByRole("table");
  await expect(table.getByRole("cell", { name: entityName, exact: true }).first()).toBeVisible();
  await expect(table.getByRole("cell", { name: bookName, exact: true }).first()).toBeVisible();

  // (b) screenshot: the booked line showing names.
  await page.screenshot({ path: `${ARTIFACTS}/rates-book-line-named.png` });

  await expectNoSeriousA11y(page, "Rates Book (named booking)");
});
