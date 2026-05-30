// Generates the Celnet "contribution" demo workbook: a single sheet wiring the
// CELNET.* custom functions (pricing/read + the MARK contribution write) so a
// trader can open it (with the add-in sideloaded) and watch live values.
// OSS only (exceljs, MIT). Run: node tools/gen-contribution.mjs
import ExcelJS from "exceljs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const out = resolve(here, "..", "contribution.xlsx");

const wb = new ExcelJS.Workbook();
wb.creator = "Celnet";
const ws = wb.addWorksheet("Celnet FX Options", {
  views: [{ showGridLines: false }],
});
ws.columns = [{ width: 22 }, { width: 30 }, { width: 30 }, { width: 18 }, { width: 14 }];

const title = (r, t) => {
  const c = ws.getCell(`A${r}`);
  c.value = t;
  c.font = { bold: true, size: 13, color: { argb: "FF1B6CF0" } };
};
const label = (cell, t) => {
  const c = ws.getCell(cell);
  c.value = t;
  c.font = { bold: true };
};
const f = (cell, formula) => {
  ws.getCell(cell).value = { formula };
};

title(1, "Celnet FX Options — live pricing & contribution");
ws.getCell("A2").value = "Add-in: CELNET.*  ·  server: ws://127.0.0.1:8081  ·  edit the inputs and watch the cells.";
ws.getCell("A2").font = { italic: true, color: { argb: "FF666666" } };

// ---- Inputs the formulas reference -----------------------------------------
label("A4", "Inputs");
label("A5", "Pair"); ws.getCell("B5").value = "EURUSD";
label("A6", "Tenor"); ws.getCell("B6").value = "1Y";
label("A7", "Strike / Delta"); ws.getCell("B7").value = "1.12";
label("A8", "Call / Put"); ws.getCell("B8").value = "C";
label("A9", "Notional"); ws.getCell("B9").value = 1000000;

// ---- Pricing / read path ----------------------------------------------------
label("A11", "Pricing (read)");
label("A12", "Price (premium)");
f("B12", 'CELNET.PRICE(B5,B6,B7,B8,B9)');
label("A13", "Live two-way (streams)");
f("B13", 'CELNET.SUBSCRIBE(B5,B6,B7,B8,B9)');
label("A15", "Greeks (spills →)");
f("B15", 'CELNET.GREEKS(B5,B6,B7,B8,B9)');
label("A18", "Surface (spills →)");
f("B18", 'CELNET.SURFACE(B5,B6)');

// ---- Contribution / write path ---------------------------------------------
label("A28", "Contribution (write)");
label("A29", "Mark pillar"); ws.getCell("B29").value = "ATM";
label("A30", "Mark vol"); ws.getCell("B30").value = 0.123;
label("A31", "Comment"); ws.getCell("B31").value = "desk mark";
label("A32", "Submit mark →");
f("B32", 'CELNET.MARK(B5,B6,B29,B30,B31)');
ws.getCell("A33").value = "(returns the new surface_version; the read cells above re-price against your mark)";
ws.getCell("A33").font = { italic: true, size: 9, color: { argb: "FF666666" } };

await wb.xlsx.writeFile(out);
console.log("wrote", out);
