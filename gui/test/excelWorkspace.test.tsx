/**
 * ExcelWorkspace — the Excel integration page. It is presentational, so the
 * load-bearing assertions are that the two get-started downloads point at the
 * served artifact paths (a wrong path 404s) and carry the `download` attribute,
 * and that the page renders its title + setup anchor.
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { ExcelWorkspace } from "../src/workspaces/ExcelWorkspace";

describe("ExcelWorkspace", () => {
  it("offers the example workbook + manifest downloads at the served paths", () => {
    render(<ExcelWorkspace />);

    expect(screen.getByRole("heading", { name: /Celnet for Excel/i })).toBeTruthy();

    const workbook = screen.getByRole("link", { name: /Download workbook/i });
    expect(workbook.getAttribute("href")).toBe("/excel/celnet-fx-options.xlsx");
    expect(workbook.hasAttribute("download")).toBe(true);

    const manifest = screen.getByRole("link", { name: /Download manifest/i });
    expect(manifest.getAttribute("href")).toBe("/excel/manifest.xml");
    expect(manifest.hasAttribute("download")).toBe(true);
  });

  it("documents the CELNET.* functions and an example formula", () => {
    render(<ExcelWorkspace />);
    // The function reference + a concrete PRICE example are present.
    expect(screen.getAllByText(/CELNET\./).length).toBeGreaterThan(0);
    // The PRICE example appears in both the example block and the setup steps.
    expect(
      screen.getAllByText(/=CELNET\.PRICE\("EURUSD","1Y",1\.12,"C",1000000\)/).length,
    ).toBeGreaterThan(0);
  });
});
