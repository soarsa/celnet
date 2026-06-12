/* TEMP: run every workbook INSTRUMENT spec through the REAL shapeSpecInstrument to
 * prove the terms build a valid instrument (a term-key typo errors here). Deleted after. */
import { readFileSync } from "node:fs";
import { shapeSpecInstrument, encodeInstrumentToken, type InstrumentSpecArgs } from "../src/functions/instrumentSpec";

interface WbInst {
  sheet: string; cell: string; underlier: string; product: string;
  terms: (string | number)[][]; tenor: string | null; notional: string | number | null;
}
const insts: WbInst[] = JSON.parse(readFileSync("/tmp/wb_instruments.json", "utf8"));
let ok = 0, err = 0;
const failures: string[] = [];
for (const i of insts) {
  const spec: InstrumentSpecArgs = {
    underlier: i.underlier,
    product: i.product,
    terms: i.terms as (string | number | boolean)[][],
    ...(i.tenor ? { tenor: i.tenor } : {}),
    ...(i.notional != null && i.notional !== "" ? { notional: Number(i.notional) } : {}),
  };
  try {
    encodeInstrumentToken(shapeSpecInstrument(spec));
    ok++;
  } catch (e) {
    err++;
    failures.push(`${i.sheet}!${i.cell}  ${i.underlier} ${i.product}  → ${(e as Error).message.slice(0, 90)}`);
  }
}
console.log(`SHAPE: ok=${ok}  err=${err}  / ${insts.length}`);
for (const f of failures) console.log("  ✗ " + f);
