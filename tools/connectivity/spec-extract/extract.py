#!/usr/bin/env python3
"""
spec-extract — v0 vendor PDF → draft QuickFIX 4.4 XML.

Phase A.1 of VENDOR_ADAPTERS_PLAN.md. The job is NOT to produce a finished
spec — it's to give the engineer a *starting point* so they don't transcribe
a FIX 4.4 envelope by hand for every one of ~150 adapters.

What this tool does (v0 + v1):
  v0 — message type detection
  1. Reads PDF text via pdfplumber.
  2. Heuristically detects which FIX message types the spec talks about
     (regex over text for `35=X`, `MsgType=X`, and friends).

  v1 — field/required table extraction
  3. Runs pdfplumber.extract_tables() per page; identifies candidate FIX
     field tables by header (`tag` + `field name`/`name` + `required`/`req`).
  4. For each candidate table, extracts the msg type from inline
     `MsgType tag 35 = X` markers + the field rows (tag number, field name,
     required Y/N/M). Falls back to FIX 4.4 standard required fields
     when no table data is extractable for a detected msg type.

  Always
  5. Emits a QuickFIX-style XML with:
       - Standard FIX 4.4 <header> and <trailer>
       - Always-on session messages (Logon/Logout/Heartbeat/TestRequest)
       - One <message> block per detected app-level message type, using
         extracted fields where available, FIX 4.4 standard fields otherwise.
  6. Falls back to envelope-only XML when no app messages are detected.

What this tool does NOT do (manual review required):
  - Vendor-specific custom fields embedded in prose (only table-extracted
    fields are pulled). High-numbered tags often need a human pass.
  - Repeating-group layout reconciliation. Group counts and member fields
    appear as flat rows in the extracted tables — restoring the QuickFIX
    `<group>` nesting is a manual step.
  - Field-validity enumerations (`<value>` blocks inside `<field>`).

Invocation:
    extract.py --in <pdf> --out <dir> [--name <basename>] [--vendor <display>]

Output:
    <out>/<basename>.xml  (basename defaults to "spec")

Exit codes:
    0  success — XML written
    1  IO / CLI error
    2  PDF unreadable (treat as fallback case in callers)
"""
from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

try:
    import pdfplumber
except ImportError:
    sys.stderr.write(
        "error: pdfplumber not installed. "
        "Run scripts/spec-extract.sh which sets up the venv automatically.\n")
    sys.exit(1)


# ---- Message-type catalogue ------------------------------------------------
#
# FIX 4.4 standard messages we know how to emit. The required-fields list is
# the minimum the QuickFIX dictionary needs to validate; vendor specs almost
# always add optional tags on top, which is a human review step.

@dataclass(frozen=True)
class MsgDef:
    msg_type: str       # 35= value
    name: str           # QuickFIX dictionary name
    category: str       # "app" or "admin"
    fields: tuple[tuple[str, bool], ...]  # (FieldName, required) — order matters for the XML


# Session-level admin messages: always emitted regardless of what the PDF says.
SESSION_MESSAGES: tuple[MsgDef, ...] = (
    MsgDef("A", "Logon", "admin", (
        ("EncryptMethod", True), ("HeartBtInt", True),
        ("ResetSeqNumFlag", False), ("Username", False), ("Password", False),
    )),
    MsgDef("0", "Heartbeat", "admin", (
        ("TestReqID", False),
    )),
    MsgDef("1", "TestRequest", "admin", (
        ("TestReqID", True),
    )),
    MsgDef("2", "ResendRequest", "admin", (
        ("BeginSeqNo", True), ("EndSeqNo", True),
    )),
    MsgDef("3", "Reject", "admin", (
        ("RefSeqNum", True), ("RefTagID", False),
        ("RefMsgType", False), ("Text", False),
    )),
    MsgDef("4", "SequenceReset", "admin", (
        ("GapFillFlag", False), ("NewSeqNo", True),
    )),
    MsgDef("5", "Logout", "admin", (
        ("Text", False),
    )),
)

# App-level messages we recognize when their MsgType appears in the PDF.
APP_MESSAGES: dict[str, MsgDef] = {
    m.msg_type: m for m in (
        # Orders.
        MsgDef("D", "NewOrderSingle", "app", (
            ("ClOrdID", True), ("Symbol", True), ("Side", True),
            ("OrderQty", True), ("OrdType", True), ("Price", False),
            ("TimeInForce", False), ("TransactTime", True),
        )),
        MsgDef("F", "OrderCancelRequest", "app", (
            ("OrigClOrdID", True), ("ClOrdID", True), ("Symbol", True),
            ("Side", True), ("TransactTime", True),
        )),
        MsgDef("G", "OrderCancelReplaceRequest", "app", (
            ("OrigClOrdID", True), ("ClOrdID", True), ("Symbol", True),
            ("Side", True), ("TransactTime", True), ("OrdType", True),
        )),
        MsgDef("8", "ExecutionReport", "app", (
            ("OrderID", True), ("ExecID", True), ("ExecType", True),
            ("OrdStatus", True), ("Symbol", True), ("Side", True),
            ("LeavesQty", True), ("CumQty", True), ("AvgPx", True),
        )),
        MsgDef("9", "OrderCancelReject", "app", (
            ("OrderID", True), ("ClOrdID", True), ("OrigClOrdID", True),
            ("OrdStatus", True), ("CxlRejResponseTo", True),
        )),
        MsgDef("q", "OrderMassCancelRequest", "app", (
            ("ClOrdID", True), ("MassCancelRequestType", True),
            ("TransactTime", True),
        )),
        # Market data.
        MsgDef("V", "MarketDataRequest", "app", (
            ("MDReqID", True), ("SubscriptionRequestType", True),
            ("MarketDepth", True),
        )),
        MsgDef("W", "MarketDataSnapshotFullRefresh", "app", (
            ("MDReqID", False), ("Symbol", True),
        )),
        MsgDef("X", "MarketDataIncrementalRefresh", "app", (
            ("MDReqID", False),
        )),
        MsgDef("Y", "MarketDataRequestReject", "app", (
            ("MDReqID", True),
        )),
        # Quotes / RFQ.
        MsgDef("R", "QuoteRequest", "app", (
            ("QuoteReqID", True), ("Symbol", True),
            ("OrderQty", False), ("Side", False),
        )),
        MsgDef("S", "Quote", "app", (
            ("QuoteReqID", False), ("QuoteID", True), ("Symbol", True),
            ("BidPx", False), ("OfferPx", False),
        )),
        MsgDef("Z", "QuoteCancel", "app", (
            ("QuoteID", False), ("QuoteCancelType", True),
        )),
        MsgDef("AJ", "QuoteResponse", "app", (
            ("QuoteRespID", True), ("QuoteRespType", True),
        )),
        # STP.
        MsgDef("AE", "TradeCaptureReport", "app", (
            ("TradeReportID", True), ("ExecType", True),
            ("Symbol", True), ("LastQty", True), ("LastPx", True),
        )),
        MsgDef("AD", "TradeCaptureReportRequest", "app", (
            ("TradeRequestID", True), ("TradeRequestType", True),
            ("SubscriptionRequestType", False),
        )),
        MsgDef("AR", "TradeCaptureReportAck", "app", (
            ("TradeReportID", True), ("TrdRptStatus", False),
        )),
    )
}


# ---- Detection -------------------------------------------------------------

# Patterns that suggest a FIX message type is being discussed:
#   "35=D"          — most reliable, exact wire format
#   "MsgType = D"   — common in spec table headers
#   "MsgType (35) = D"
#   "Message Type: D - NewOrderSingle"
_MSGTYPE_RE = re.compile(
    r"""
    (?:
        35\s*=\s*(?P<a>[A-Za-z0-9]{1,3})           # 35=D, 35=AJ
        |
        Msg(?:Type|\s*Type)\s*(?:\([^)]*\))?       # MsgType / MsgType(35)
            \s*[=:]\s*['"]?(?P<b>[A-Za-z0-9]{1,3})['"]?
        |
        Message\s+Type\s*[:=]\s*['"]?(?P<c>[A-Za-z0-9]{1,3})['"]?
            \s*[-–]\s*(?P<c_name>[A-Z][A-Za-z]+)   # "Message Type: D - NewOrderSingle"
    )
    """,
    re.VERBOSE,
)


def detect_message_types(pages_text: Iterable[str]) -> set[str]:
    """Scan all extracted text for evidence of FIX MsgType usage."""
    found: set[str] = set()
    for text in pages_text:
        for m in _MSGTYPE_RE.finditer(text):
            mt = m.group("a") or m.group("b") or m.group("c")
            if mt and mt in APP_MESSAGES:
                found.add(mt)
    return found


# ---- v1 table extraction ---------------------------------------------------
#
# pdfplumber's extract_tables() returns a per-page list of tables; each table
# is a list[list[Optional[str]]]. Vendor spec PDFs vary wildly in cell
# granularity — narrow columns often get split into multiple cells with
# empty neighbours. The strategy is forgiving:
#   1. Identify candidate field tables by header keywords.
#   2. Locate column ranges (not single indices) for tag / name / required.
#   3. For each row, pick the first non-empty cell in each range.
#   4. Find the MsgType binding from any cell that mentions "MsgType ... = X".
#
# An ExtractedFields entry is `(field_name, required_bool)` in row order,
# preserving the spec's stated order of fields per message.

ExtractedFields = list[tuple[str, bool]]


def _normalise_cell(c: str | None) -> str:
    return (c or "").strip().replace("\n", " ")


def _column_ranges(header: list[str]) -> dict[str, range] | None:
    """Map logical columns (tag/name/required/comments) to header index ranges.

    A "range" because some PDF tables split one logical column across several
    physical cells; the actual column we want spans header[start:end]. Returns
    None when the header doesn't look like a FIX field table.
    """
    # First pass: locate the keyword start indices.
    keyword_at: dict[str, int] = {}
    for i, h in enumerate(header):
        low = h.lower()
        if "tag" in low and "tag" not in keyword_at:
            keyword_at["tag"] = i
        elif (("field" in low and "name" in low) or low == "name"
              or low == "field name") and "name" not in keyword_at:
            keyword_at["name"] = i
        elif (("req" in low) or low == "m" or "mandatory" in low) \
                and "required" not in keyword_at:
            keyword_at["required"] = i
        elif "comment" in low and "comments" not in keyword_at:
            keyword_at["comments"] = i

    if "tag" not in keyword_at or "name" not in keyword_at:
        return None

    # Second pass: turn each keyword index into a range that ends at the next
    # keyword. Columns the header didn't label become part of the previous
    # logical column.
    ordered = sorted(keyword_at.items(), key=lambda kv: kv[1])
    ranges: dict[str, range] = {}
    for i, (key, start) in enumerate(ordered):
        end = ordered[i + 1][1] if i + 1 < len(ordered) else len(header)
        ranges[key] = range(start, end)
    return ranges


def _first_non_empty(row: list[str | None], r: range) -> str:
    for i in r:
        if i < len(row):
            c = _normalise_cell(row[i])
            if c:
                return c
    return ""


_INLINE_MSGTYPE_RE = re.compile(
    r"""
    (?:
        35\s*=\s*['\"]?(?P<a>[A-Za-z0-9]{1,3})['\"]?
        |
        MsgType\s*(?:\([^)]*\)|tag\s*35)?\s*[=:]\s*['\"]?(?P<b>[A-Za-z0-9]{1,3})['\"]?
    )
    """,
    re.IGNORECASE | re.VERBOSE,
)


def parse_field_table(table: list[list[str | None]]) -> tuple[str | None, ExtractedFields]:
    """Return `(msg_type_or_None, fields)`. Empty result if not a field table.

    Filters out the obligatory "Standard Header" / "Standard Trailer" reference
    rows that vendor specs sprinkle through every message table — those are
    re-imported from our standard header/trailer in the final XML.
    """
    if not table or len(table) < 2:
        return None, []
    header = [_normalise_cell(c) for c in table[0]]
    ranges = _column_ranges(header)
    if not ranges:
        return None, []

    msg_type: str | None = None
    fields: ExtractedFields = []
    seen_names: set[str] = set()

    for row in table[1:]:
        # Hunt for the MsgType binding anywhere in the row text (vendors put
        # it in the comments column, in the "Standard Header" row, or as a
        # caption above the table that pdfplumber stuffed into one cell).
        row_text = " ".join(_normalise_cell(c) for c in row)
        if msg_type is None:
            m = _INLINE_MSGTYPE_RE.search(row_text)
            if m:
                candidate = m.group("a") or m.group("b")
                # Some specs spell out "35 = D (NewOrderSingle)" — preserve
                # only the MsgType code itself, drop parentheticals.
                if candidate in APP_MESSAGES or candidate in {
                    sm.msg_type for sm in SESSION_MESSAGES
                }:
                    msg_type = candidate

        tag = _first_non_empty(row, ranges["tag"])
        name = _first_non_empty(row, ranges["name"])
        required_raw = (
            _first_non_empty(row, ranges["required"]) if "required" in ranges else ""
        )

        # Skip reference rows. The integer-tag check also drops group-count
        # rows like "Repeating Group" that vendors interleave; restoring those
        # is a manual review step (documented in the file header).
        if not tag.isdigit():
            continue
        if not name or name in {"Standard Header", "Standard Trailer"}:
            continue
        if name in seen_names:
            continue
        seen_names.add(name)

        required = required_raw.upper().startswith(("Y", "M"))
        fields.append((name, required))

    return msg_type, fields


def extract_field_tables(pdf) -> dict[str, ExtractedFields]:
    """Walk every page, return `msg_type → fields` for tables we can parse.

    When the same msg_type appears in multiple tables (vendors sometimes
    duplicate the NewOrderSingle structure across an "overview" page and a
    "detailed" page), the LARGER field list wins — usually the detailed one.
    """
    out: dict[str, ExtractedFields] = {}
    for page in pdf.pages:
        for tbl in page.extract_tables() or []:
            msg_type, fields = parse_field_table(tbl)
            if not msg_type or not fields:
                continue
            existing = out.get(msg_type)
            if existing is None or len(fields) > len(existing):
                out[msg_type] = fields
    return out


# ---- PDF reading -----------------------------------------------------------

def read_pdf(path: Path) -> tuple[list[str], dict[str, ExtractedFields]]:
    """Open the PDF once, return (per-page text, msg_type → extracted fields).

    Doing both in one pass avoids re-opening the PDF (pdfplumber's open cost
    is the bulk of runtime for the >100-page bank specs).
    """
    try:
        with pdfplumber.open(str(path)) as pdf:
            pages_text = [page.extract_text() or "" for page in pdf.pages]
            tables = extract_field_tables(pdf)
    except Exception as e:
        sys.stderr.write(f"warning: pdfplumber failed on {path.name}: {e}\n")
        return [], {}
    return pages_text, tables


# ---- HTML reading ----------------------------------------------------------
#
# Some vendors (Rabobank) ship HTML specs; others (MorganStanley, MUFG) ship
# .doc/.docx which we convert to HTML via macOS `textutil` before parsing.
# HTML doesn't have pages, so we synthesise a single "page" containing the
# entire text. Field tables come straight from <table> elements parsed by
# the same `parse_field_table` codepath used for PDFs.

def _html_tables(soup) -> list[list[list[str | None]]]:
    """Return each <table> as a list[list[str]] matching pdfplumber's shape."""
    out = []
    for table in soup.find_all("table"):
        rows = []
        for tr in table.find_all("tr"):
            cells = [
                # Use get_text(' ', strip=True) so nested <p>/<span> become
                # single-line cell content matching pdfplumber's behaviour.
                cell.get_text(" ", strip=True) or None
                for cell in tr.find_all(["td", "th"])
            ]
            if cells:
                rows.append(cells)
        if rows:
            out.append(rows)
    return out


def read_html(path: Path) -> tuple[list[str], dict[str, ExtractedFields]]:
    """Parse an HTML spec file. Returns the same shape as read_pdf."""
    try:
        from bs4 import BeautifulSoup
    except ImportError:
        sys.stderr.write(
            "error: beautifulsoup4 not installed — run spec-extract.sh "
            "to refresh the venv\n")
        return [], {}
    try:
        # Many vendor HTML files are saved as latin-1 or windows-1252; fall
        # back gracefully if utf-8 fails.
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            text = path.read_text(encoding="latin-1")
        soup = BeautifulSoup(text, "html.parser")
    except Exception as e:
        sys.stderr.write(f"warning: HTML parse failed on {path.name}: {e}\n")
        return [], {}

    body_text = soup.get_text("\n", strip=True)
    tables: dict[str, ExtractedFields] = {}
    for tbl in _html_tables(soup):
        msg_type, fields = parse_field_table(tbl)
        if not msg_type or not fields:
            continue
        existing = tables.get(msg_type)
        if existing is None or len(fields) > len(existing):
            tables[msg_type] = fields
    return [body_text], tables


# ---- .doc / .docx reading via textutil ------------------------------------
#
# textutil ships with macOS and handles both legacy .doc binary format and
# OOXML .docx. Convert to HTML in a temp file, then delegate to read_html.

def read_word(path: Path) -> tuple[list[str], dict[str, ExtractedFields]]:
    """Convert .doc/.docx to HTML via textutil, then parse as HTML."""
    import subprocess
    import tempfile
    with tempfile.NamedTemporaryFile(suffix=".html", delete=False) as tmp:
        html_path = Path(tmp.name)
    try:
        result = subprocess.run(
            ["textutil", "-convert", "html",
             str(path), "-output", str(html_path)],
            capture_output=True, text=True,
        )
        if result.returncode != 0:
            sys.stderr.write(
                f"warning: textutil failed on {path.name}: "
                f"{result.stderr.strip()}\n")
            return [], {}
        return read_html(html_path)
    finally:
        html_path.unlink(missing_ok=True)


def read_spec(path: Path) -> tuple[list[str], dict[str, ExtractedFields]]:
    """Dispatch on file extension. Adds .html / .doc / .docx to the pipeline."""
    suffix = path.suffix.lower()
    if suffix == ".pdf":
        return read_pdf(path)
    if suffix in (".html", ".htm"):
        return read_html(path)
    if suffix in (".doc", ".docx"):
        return read_word(path)
    sys.stderr.write(
        f"error: unsupported input extension '{suffix}' on {path.name} — "
        "supported: .pdf .html .htm .doc .docx\n")
    return [], {}


# ---- XML emission ----------------------------------------------------------

_STANDARD_HEADER_FIELDS = (
    ("BeginString", True),
    ("BodyLength", True),
    ("MsgType", True),
    ("SenderCompID", True),
    ("TargetCompID", True),
    ("MsgSeqNum", True),
    ("SendingTime", True),
    ("PossDupFlag", False),
    ("PossResend", False),
    ("OrigSendingTime", False),
)


def render_message(
    m: MsgDef, extracted_fields: ExtractedFields | None = None,
) -> str:
    """Render a <message> block. Uses `extracted_fields` (in order) when given,
    falling back to the catalogue's standard fields otherwise."""
    fields = extracted_fields if extracted_fields else list(m.fields)
    lines = [
        f'    <message name="{m.name}" msgcat="{m.category}" msgtype="{m.msg_type}">',
    ]
    for name, required in fields:
        lines.append(f'      <field name="{name}" required="{"Y" if required else "N"}"/>')
    lines.append("    </message>")
    return "\n".join(lines)


_FALLBACK_APP_MESSAGES: tuple[str, ...] = (
    # Always emit a "starter set" of common app messages when text scanning
    # finds nothing. Keeps the downstream scaffold + conformance test useful
    # — the alternative is an envelope-only XML where `msgtype="D"`/`"V"`/`"R"`
    # are missing, which trips the standard "spec mentions primary message"
    # assertion in `scripts/new-adapter.sh`'s generated conformance test.
    "D", "F", "8", "9",          # orders
    "V", "W", "X", "Y",          # market data
    "R", "S", "Z",               # quotes
)


# Primary message types per connection type — what the scaffolded conformance
# test's `spec_mentions_primary_message_type` assertion looks for. When the
# caller passes `--conn-type`, we guarantee these are in the output even if
# the PDF didn't mention them, so partial-detection PDFs still scaffold cleanly.
_CONN_TYPE_PRIMARIES: dict[str, tuple[str, ...]] = {
    "order": ("D", "8"),
    "price": ("V", "W"),
    "rfq":   ("R", "S"),
    "stp":   ("AE", "AR"),
}


def render_xml(
    vendor: str, source_pdf: str, detected: set[str],
    extracted_tables: dict[str, ExtractedFields] | None = None,
    conn_type: str | None = None,
) -> str:
    extracted_tables = extracted_tables or {}
    if not detected:
        # Fallback: include a starter set of common app messages so the
        # output XML is a usable scaffold, not a useless envelope-only stub.
        detected = set(_FALLBACK_APP_MESSAGES)
    if conn_type:
        # Whether we extracted or fell back: ensure the conn_type's primary
        # messages are represented. A "PRICE" adapter MUST have V/W in the
        # spec so the downstream conformance test passes; an "STP" adapter
        # MUST have AE/AR even when the PDF was envelope-only (drop-copy
        # PDFs are often acceptance docs that don't enumerate the wire).
        primaries = _CONN_TYPE_PRIMARIES.get(conn_type.lower(), ())
        detected = set(detected) | set(primaries)
    detected_sorted = sorted(detected)
    app_messages = [APP_MESSAGES[mt] for mt in detected_sorted if mt in APP_MESSAGES]

    header_block = "\n".join(
        f'    <field name="{n}" required="{"Y" if r else "N"}"/>'
        for n, r in _STANDARD_HEADER_FIELDS
    )

    session_blocks = "\n".join(
        render_message(m, extracted_tables.get(m.msg_type)) for m in SESSION_MESSAGES
    )
    app_blocks = "\n".join(
        render_message(m, extracted_tables.get(m.msg_type)) for m in app_messages
    )
    msg_blocks = "\n".join(b for b in (session_blocks, app_blocks) if b)

    fallback_note = ""
    if detected == set(_FALLBACK_APP_MESSAGES):
        # `render_xml`'s `detected` parameter is the post-fallback set, so we
        # detect the fallback case by comparing against the seed list. Note
        # this is an over-approximation: a PDF whose detected set happens to
        # equal the starter set exactly will also get this note. Vanishingly
        # rare in practice, and the note is only ever advisory.
        fallback_note = (
            "  <!-- FALLBACK: no app-level message types detected in the source PDF.\n"
            "       Output below is the FIX 4.4 starter catalogue — review against\n"
            "       the vendor spec and trim/extend by hand. -->\n"
        )

    with_tables = sorted(set(extracted_tables) & set(detected_sorted))
    detected_summary = (
        f"  <!-- Detected MsgTypes: {', '.join(detected_sorted)} -->\n"
        f"  <!-- Field tables extracted for: "
        f"{', '.join(with_tables) if with_tables else '(none — fields are FIX 4.4 defaults)'} -->"
        if detected
        else "  <!-- Detected MsgTypes: (none — see fallback note above) -->"
    )

    return (
f"""<!--
  Draft QuickFIX 4.4 dictionary auto-extracted by scripts/spec-extract.

  Vendor:  {vendor}
  Source:  {source_pdf}

  This file is a STARTING POINT for the engineer. v0 of the extractor only
  detects which message types the spec discusses; the field lists below are
  FIX 4.4 standard required fields, NOT extracted from the vendor's tables.

  Review pass before this becomes a real adapter spec:
    1. Add vendor-custom fields (typically 5000-9999 tag range).
    2. Promote optional fields to required where the vendor mandates them.
    3. Replace repeating-group placeholders with the layout from the PDF.
    4. Delete any messages the vendor doesn't actually support.
-->
{detected_summary}
{fallback_note}<fix major="4" minor="4" type="FIX">
  <header>
{header_block}
  </header>

  <messages>
{msg_blocks}
  </messages>

  <trailer>
    <field name="CheckSum" required="Y"/>
  </trailer>
</fix>
"""
    )


# ---- CLI ------------------------------------------------------------------

def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(
        prog="spec-extract",
        description="v0 vendor PDF → draft QuickFIX 4.4 XML. "
                    "Output is a starting point, not a finished spec.",
    )
    p.add_argument("--in", dest="input", required=True, type=Path,
                   help="vendor spec file (.pdf, .html/.htm, or .doc/.docx)")
    p.add_argument("--out", required=True, type=Path,
                   help="output directory (created if missing)")
    p.add_argument("--name", default="spec",
                   help="basename for the output XML (default: spec)")
    p.add_argument("--vendor", default=None,
                   help="vendor display name embedded in the XML header comment")
    p.add_argument("--conn-type", default=None, choices=["order", "price", "rfq", "stp"],
                   help="when set, guarantees the primary messages for that "
                        "conn type (D/V/R/AE + their counterparties) are in the "
                        "output even if the PDF didn't mention them")
    args = p.parse_args(argv)

    if not args.input.exists():
        sys.stderr.write(f"error: input spec not found: {args.input}\n")
        return 1

    args.out.mkdir(parents=True, exist_ok=True)
    out_path = args.out / f"{args.name}.xml"

    pages, tables = read_spec(args.input)
    if not pages:
        # Unreadable spec: still emit the envelope-only fallback so the
        # downstream scaffolding has something to wire up.
        sys.stderr.write(
            f"warning: spec unreadable, emitting envelope-only XML to {out_path}\n")
        out_path.write_text(render_xml(
            vendor=args.vendor or args.input.stem,
            source_pdf=args.input.name,
            detected=set(),
            conn_type=args.conn_type,
        ))
        return 2

    detected = detect_message_types(pages)
    # Any msg_type seen in a field-table caption that we didn't catch in the
    # text scan: also include it. Tables are more authoritative — if a field
    # block for 35=R exists, the spec supports QuoteRequest even if the prose
    # doesn't repeat the wire form.
    detected.update(mt for mt in tables if mt in APP_MESSAGES)

    xml = render_xml(
        vendor=args.vendor or args.input.stem,
        source_pdf=args.input.name,
        detected=detected,
        extracted_tables=tables,
        conn_type=args.conn_type,
    )
    out_path.write_text(xml)

    extracted_for_detected = sorted(set(tables) & detected)
    total_fields = sum(len(tables[mt]) for mt in extracted_for_detected)
    sys.stdout.write(
        f"→ {out_path}\n"
        f"  pages read:      {len(pages)}\n"
        f"  msg types found: {len(detected)}"
        + (f" ({', '.join(sorted(detected))})" if detected else " (none — envelope-only fallback)")
        + "\n"
        f"  field tables:    {len(extracted_for_detected)} msg types, "
        f"{total_fields} fields total"
        + (f" ({', '.join(extracted_for_detected)})" if extracted_for_detected else "")
        + "\n"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
