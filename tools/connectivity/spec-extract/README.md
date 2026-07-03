# spec-extract — vendor FIX-spec PDF → draft QuickFIX 4.4 XML

A starting-point generator for the CelNet connectivity adapter backlog. It turns
a vendor's FIX specification PDF into a *draft* QuickFIX-style XML dictionary so
an engineer doesn't hand-transcribe a FIX 4.4 envelope for every venue. Ported
verbatim from the `soarsa/celnet-connectivity` review (OSS: pdfplumber +
beautifulsoup4). Feeds `crates/celnet-connectivity/specs/`.

## Install & run

```bash
cd tools/connectivity/spec-extract
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt              # pdfplumber, beautifulsoup4 (OSS)
python3 extract.py --in <vendor-spec.pdf> --out ../../../crates/celnet-connectivity/specs/ \
                   --name fix44_<vendor> --vendor "<Display Name>" --conn-type order
pytest test_extract.py                        # the tool's own test suite
```

`--conn-type` ∈ `order | price | rfq | stp` selects which app messages to seed.

## What it does

1. Reads the PDF (pdfplumber), heuristically detects which FIX message types the
   spec references (`35=X` / `MsgType=X`).
2. Extracts candidate FIX field tables (tag / name / required) per page.
3. Emits QuickFIX-style XML: standard FIX 4.4 header/trailer + always-on session
   messages (Logon/Logout/Heartbeat/TestRequest) + one `<message>` per detected
   app message, using extracted fields where available and FIX 4.4 standard
   fields otherwise. Falls back to an envelope-only XML when nothing is detected.

## What it does NOT do (a human review pass is required)

- Vendor-custom fields buried in prose (only table-extracted fields are pulled).
- Repeating-group `<group>` nesting (extracted as flat rows — restore by hand).
- Field-validity `<value>` enumerations inside `<field>`.

The output is an **80%-there draft**, not a finished spec — always diff it
against the vendor PDF before committing the dictionary.

> Note: the CelNet gate runs cargo (`just check`), which does not exercise this
> Python tool. Run `pytest test_extract.py` inside the venv to validate changes
> to the extractor itself.
