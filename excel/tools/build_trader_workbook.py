#!/usr/bin/env python3
"""Celnet trader workbook — a multi-asset options desk's single trading day.

State-of-the-art, by-asset-class, competitor-beating demo for Excel users:
the workbook IS one trading day on the Celnet multi-asset options desk, told
tab-by-tab as the sun moves across the desks (Cover -> Market & Vol -> FX
Majors -> FX EM & NDF -> Metals -> Equity -> Commodity -> Crypto -> Cross-Asset
RV -> Risk Cockpit). Every tab is a scenario a trader actually runs, never a
function catalogue.

Celnet dark theme (memory: brand-kit): deep-navy canvas, coral signature +
indigo interactive accent, the geometric mark + Anaheim "Celnet"
wordmark band, the 6px coral cap-rail, the build-stamp footer.

Uses ONLY the 13 registered custom functions (INSTRUMENT, PRICE, GREEKS, RFQ,
SUBSCRIBE, SURFACE, MARKSURFACE, SERIES, MARK, RISK, POSITIONS, LIMITS, STATUS).
Every exotic / strategy / cross-asset structure is built from one polymorphic
CELNET.INSTRUMENT(underlier, product, <terms_range>, [tenor], [notional]) token
whose terms live in an EDITABLE 2-column key/value cell range — the proven
"bump-and-watch" pattern — and the verbs (PRICE/GREEKS/RFQ/SUBSCRIBE) reference
the INSTRUMENT token cell.

Every CELNET.* formula is written PLAIN via cf() — NO `_xlfn.` prefix. Office.js
add-in custom functions resolve as plain NAMESPACE.NAME; the `_xlfn.` prefix is
only for built-ins and #NAME?s a typed `=_xlfn.CELNET.STATUS()`.

The hard capability matrix is enforced in ONE place (the desk-sheet factory):
FX & METAL underliers may use any of the 24 arms; EQUITY / COMMODITY / CRYPTO are
cost-of-carry leaves — only VANILLA / PERPETUAL / FUTUREOPTION are priceable. The
ONE intentional exception is the Equity tab's labelled CAPABILITY-WALL cell (an
equity BARRIER), a deliberate demonstration that the build-time guard returns an
honest typed error rather than a silent #N/A.

    /tmp/celnet-xlsx-venv/bin/python excel/tools/build_trader_workbook.py [out.xlsx]
"""
import os
import shutil
import sys

import xlsxwriter

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/Desktop/Celnet-Trader.xlsx")

# ---- LOGO: resolve logo.png robustly (repo-root, relative to script) -
_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
_REPO_ROOT = os.path.abspath(os.path.join(_SCRIPT_DIR, "..", ".."))
LOGO = ""
for _cand in (
    os.path.join(_REPO_ROOT, "logo.png"),
    os.path.join(_SCRIPT_DIR, "logo.png"),
    "/tmp/logo.png",
):
    if os.path.exists(_cand):
        # Stage to /tmp so xlsxwriter's image embed always reads a stable path,
        # and so a read-only checkout location never blocks the run.
        try:
            if os.path.abspath(_cand) != os.path.abspath(LOGO):
                shutil.copyfile(_cand, LOGO)
        except OSError:
            LOGO = _cand
        break
# If none exist LOGO points at a non-existent /tmp path; brand() guards os.path.exists.

# ---- Dark palette (memory: brand-kit) --------------------------
CANVAS = "#161A24"   # deep navy canvas
BAND = "#1B1F2A"     # brand band (= logo bg, seamless)
PANEL = "#1F2430"    # raised panel
RAISED = "#2A3040"   # title bar
CORAL = "#FF7357"    # signature
INDIGO = "#6B6BF5"   # interactive accent
INKL = "#E8EAF2"     # primary text on dark
SECON = "#9AA0BF"    # secondary / labels
INPUT = "#242C40"    # editable fill
GRID = "#2C3242"     # hairlines
GREEN = "#1B7F4B"    # FX desk tab
GOLD = "#B8860B"     # metals tab
EQBLUE = "#3A4DB0"   # equity tab
COMMBR = "#8A6D3B"   # commodity tab
VIOLET = "#8A2BE2"   # cross-asset / risk tab
RAG_R, RAG_A, RAG_G = "#7A1F2B", "#7A5A1F", "#1F5A33"  # diverging heat anchors
FONT, MONO = "Anaheim", "Menlo"

wb = xlsxwriter.Workbook(OUT, {"nan_inf_to_errors": True})
wb.set_calc_mode("auto")


def cf(f):
    # Office.js add-in custom functions resolve as plain NAMESPACE.NAME (no _xlfn.
    # prefix — that's only for built-ins). Empirically confirmed: a typed
    # =CELNET.STATUS() resolves while =_xlfn.CELNET.STATUS() #NAME?s. cf() is the
    # single pass-through every CELNET.* formula goes through.
    return f


def fmt(**kw):
    base = {"font_name": FONT, "font_color": INKL, "bg_color": CANVAS, "valign": "vcenter"}
    base.update(kw)
    return wb.add_format(base)


S = {
    "canvas": fmt(),
    "word": fmt(bold=True, font_size=20, font_color="#FFFFFF", bg_color=BAND),
    "wsub": fmt(font_size=9, font_color=SECON, bg_color=BAND),
    "band": fmt(bg_color=BAND),
    "coral": fmt(bg_color=CORAL),
    "title": fmt(bold=True, font_size=11, font_color="#FFFFFF", bg_color=RAISED, indent=1),
    "stamp": fmt(font_name=MONO, font_size=8, font_color=SECON, align="right"),
    "secI": fmt(bold=True, font_size=10, font_color="#FFFFFF", bg_color=INDIGO, indent=1),
    "secC": fmt(bold=True, font_size=10, font_color="#161A24", bg_color=CORAL, indent=1),
    "lab": fmt(bold=True, font_size=9, font_color=SECON),
    "labr": fmt(bold=True, font_size=9, font_color=SECON, align="right"),
    "note": fmt(italic=True, font_size=9, font_color=SECON, text_wrap=True, valign="top"),
    "noteC": fmt(italic=True, font_size=9, font_color=CORAL, text_wrap=True, valign="top"),
    "hero": fmt(bold=True, font_size=16, font_color="#FFFFFF", bg_color=CANVAS),
    "heroS": fmt(font_size=11, font_color=CORAL, bg_color=CANVAS),
    "link": fmt(font_size=10, font_color=INDIGO, underline=1),
    "step": fmt(bold=True, font_size=9, font_color="#161A24", bg_color=INDIGO, align="center"),
    "ul": fmt(bold=True, font_size=8, font_color=SECON, align="center", bottom=1, bottom_color=GRID),
    "ull": fmt(bold=True, font_size=8, font_color=SECON, bottom=1, bottom_color=GRID),
    "pair": fmt(bold=True, font_size=10, font_color="#FFFFFF"),
    "val": fmt(font_name=MONO, font_size=10, font_color=INKL),
    "tok": fmt(font_name=MONO, font_size=8, font_color=SECON, text_wrap=False),
    "num": fmt(font_name=MONO, font_size=10, font_color=INKL, num_format="0.0000", align="center"),
    "pct": fmt(font_name=MONO, font_size=10, font_color=INKL, num_format="0.00", align="center"),
    "cor": fmt(font_name=MONO, font_size=10, font_color=CORAL, num_format="0.0000", align="center"),
    "axe": fmt(bold=True, font_size=9, font_color=CORAL, align="center"),
    "cd": fmt(font_name=MONO, font_size=9, font_color=SECON, num_format="dd-mmm-yy", align="center"),
    "in": fmt(font_size=10, font_color="#FFFFFF", bg_color=INPUT, align="center", bottom=2, bottom_color=CORAL),
    "inl": fmt(font_size=10, font_color="#FFFFFF", bg_color=INPUT, bottom=2, bottom_color=CORAL),
    "inp": fmt(font_size=10, font_color="#FFFFFF", bg_color=INPUT, num_format="0.00", align="center", bottom=2, bottom_color=CORAL),
    "inc": fmt(font_size=10, font_color="#FFFFFF", bg_color=INPUT, num_format="#,##0", align="center", bottom=2, bottom_color=CORAL),
    "ink": fmt(font_size=9, font_color=SECON, bg_color=INPUT, bottom=1, bottom_color=GRID),  # term key
    "date": fmt(font_size=10, font_color="#FFFFFF", bg_color=INPUT, num_format="dd-mmm-yyyy", align="center", bottom=2, bottom_color=CORAL),
}


def setup(ws, widths):
    ws.hide_gridlines(2)
    ws.set_paper(9)
    ws.set_landscape()
    for c, w in enumerate(widths):
        ws.set_column(c, c, w, S["canvas"])
    ws.set_column(len(widths), 40, 11, S["canvas"])  # extend the dark canvas right


def brand(ws, title, n):
    ws.set_row(0, 6)
    ws.merge_range(0, 0, 0, n, "", S["coral"])
    ws.set_row(1, 46)
    ws.merge_range(1, 0, 1, n, "", S["band"])
    if os.path.exists(LOGO):
        ws.insert_image(1, 0, LOGO, {"x_offset": 14, "y_offset": 7, "x_scale": 0.165, "y_scale": 0.165})
    ws.write_string(1, 1, "Celnet", S["word"])
    ws.set_row(2, 4)
    ws.merge_range(2, 0, 2, n, "", S["band"])
    ws.set_row(3, 20)
    ws.merge_range(3, 0, 3, n, "   " + title.upper(), S["title"])
    ws.freeze_panes(4, 0)


def sec(ws, r, text, n, coral=False):
    ws.set_row(r, 18)
    ws.merge_range(r, 0, r, n, "  " + text.upper(), S["secC"] if coral else S["secI"])


def stamp(ws, r, n):
    ws.set_row(r, 16)
    ws.merge_range(r, 0, r, n,
                   "CELNET v1.0-RC    ws://127.0.0.1:8081    GUI · SDK · CLI · EXCEL PARITY   ",
                   S["stamp"])


def wf(ws, r, c, f, key):
    ws.write_formula(r, c, cf(f), S[key])


# ===========================================================================
#  Term-matrix machinery (HARD RULE 1): write structure terms as adjacent
#  key/value cells, then reference that range from CELNET.INSTRUMENT — never an
#  inline {...} brace literal (xlsxwriter would treat it as an array constant).
# ===========================================================================

def write_terms(ws, r0, kcol, terms):
    """Write a 2-column key/value terms block starting at (r0, kcol).

    `terms` is a list of (key, value) scalar pairs and/or ("legs", [c0, c1, ...])
    matrix rows (the strategy/basket repeated-key form: one row per leg). Returns
    the A1 range string spanning keys (kcol) and the widest value span — exactly
    what CELNET.INSTRUMENT's matrix `terms` argument consumes.
    """
    r = r0
    maxvcol = kcol + 1
    for key, val in terms:
        ws.write(r, kcol, key, S["ink"])
        if isinstance(val, (list, tuple)):
            # Repeated matrix row (legs / correlations): payload cells to the right.
            for j, cell in enumerate(val):
                vk = "ink" if isinstance(cell, str) else "in"
                ws.write(r, kcol + 1 + j, cell, S[vk])
            maxvcol = max(maxvcol, kcol + len(val))
        else:
            vk = "inl" if isinstance(val, str) else ("inc" if abs(val) >= 1000 else "inp")
            ws.write(r, kcol + 1, val, S[vk])
        r += 1
    r1 = r - 1
    a = xlsxwriter.utility.xl_rowcol_to_cell(r0, kcol)
    b = xlsxwriter.utility.xl_rowcol_to_cell(r1, maxvcol)
    return f"{a}:{b}", r1


# ===========================================================================
#  desk_sheet(...) — the column-stable, capability-aware scenario-ladder factory
#  shared by FX Majors / FX EM & NDF / Metals / Equity / Commodity / Crypto /
#  Cross-Asset RV. One function lays the identical shape so the eye lands on the
#  same cell on every desk, and the capability matrix is enforced in ONE place.
# ===========================================================================

# The cost-of-carry leaves: an equity/commodity/crypto underlier supports ONLY
# these arms (mirrors instrumentSpec.ts shapeSpecInstrument's build-time guard).
LEAF_ARMS = {"VANILLA", "PERPETUAL", "FUTUREOPTION"}


def _is_cross_asset(underlier):
    """True for equity/commodity/crypto underliers (…@… or …/… or :suffix)."""
    u = underlier.upper()
    return ("@" in u) or ("/" in u) or u.endswith(":INVERSE") or u.endswith(":LINEAR")


def _leaf_class(underlier):
    u = underlier.upper()
    if "@" in u:
        return "commodity" if u.split("@", 1)[1].startswith(":") else "equity"
    if "/" in u or u.endswith(":INVERSE") or u.endswith(":LINEAR"):
        return "crypto"
    return None


# ---------------------------------------------------------------------------
#  Column geometry of a desk sheet (column-stable across every desk).
#
#  The 2024-06-12 collision-free redesign: the INSTRUMENT token sits at a FIXED
#  column PAST the widest terms matrix, and the ladder verbs are NON-SPILLING
#  SCALARS (INDEX over the dynamic array) so nothing ever overlaps a neighbour:
#
#    A(0)  STRUCTURE label
#    B(1)  PRODUCT
#    C..G (2..6)  TERMS key/value matrix  (key=C; up to 4 payload cells D..G —
#                 covers a 4-field strategy leg [callPut,strike,side,ratio])
#    H(7)  INSTRUMENT TOKEN   ← fixed, always clear of the terms matrix
#    I(8)  PREMIUM  = INDEX(CELNET.PRICE(tok),1,2)     (scalar, coral)
#    J(9)  Δ        = INDEX(CELNET.GREEKS(tok),1,2)    (delta_spot, scalar)
#    K(10) ν        = INDEX(CELNET.GREEKS(tok),4,2)    (vega, scalar)
#    L(11) STREAM   = CELNET.SUBSCRIBE(tok)            (one cell — fine)
#
#  None of I/J/K/L spills, so adjacent rows/columns never collide. The rich
#  PRICE/RFQ spills are showcased ONCE per sheet in the FULL RISK / TERM SHEET
#  block, each given clear rows and columns.
# ---------------------------------------------------------------------------
TOK_COL = 7      # H — the INSTRUMENT token, fixed past the C..G terms matrix
PREM_COL = 8     # I — premium scalar (coral)
DELTA_COL = 9    # J — delta_spot scalar
VEGA_COL = 10    # K — vega scalar
STREAM_COL = 11  # L — SUBSCRIBE (one cell)
DESK_N = 11      # merges (sections / brand / notes / stamp) span A..L


def _full_risk_block(ws, r, n, headline):
    """ONE featured FULL RISK / TERM SHEET per desk: the section headline token's
    complete PRICE spill + RFQ spill, each given ~18 clear rows and clear columns
    so the rich dynamic arrays are showcased with room and never overlap.

    `headline` is (label, token_cell_A1) — the token cell whose verbs we spill.
    """
    label, tok = headline
    sec(ws, r, "Full risk / term sheet — the headline structure, in full", n, coral=True)
    r += 1
    ws.write(r, 0, label, S["pair"])
    # Two spill anchors on the SAME row, columns far apart so the (≤2-col) PRICE
    # spill (col A→B, ~16 rows) never reaches the RFQ spill (col F→I, ~few rows).
    ws.write(r + 1, 0, "PRICE(token) ↓ premium + 13 Greeks + convention footer", S["lab"])
    wf(ws, r + 2, 0, f"=CELNET.PRICE({tok})", "cor")
    ws.write(r + 1, 5, "RFQ(token) ↓ two-way + dealer panel + footer", S["lab"])
    wf(ws, r + 2, 5, f"=CELNET.RFQ({tok})", "val")
    # Reserve 18 clear rows below the anchors for the taller of the two spills.
    return r + 2 + 18


def desk_sheet(name, tab_color, title, n, ctx, sections, live, notes=None,
               allow_wall=False, extra=None):
    """Build one column-stable desk sheet.

    ctx      : list of (label, value, fmtkey) input cells laid across the top.
    sections : list of dicts:
                 {"title": str, "coral": bool, "rows": [structure, ...]}
               a structure is a dict:
                 {"label", "underlier"|"underlier_cell", "product",
                  "terms": [(k,v)...], "tenor": str|None, "notional": cell|num|None,
                  "verbs": ["PRICE","GREEKS","RFQ","SUBSCRIBE"],
                  "wall": bool (the labelled capability-wall demo; bypasses guard)}
    live     : list of (label, formula) rows for the right-hand LIVE strip.
    notes    : list of (text, coral) annotation lines under the ladder.

    The verbs in the ladder are NON-SPILLING scalars (INDEX over the dynamic
    array); the parameter `n` is ignored for width (every desk uses the fixed
    DESK_N geometry) but kept for the call-site signature.
    """
    n = DESK_N
    ws = wb.add_worksheet(name)
    ws.set_tab_color(tab_color)
    #        A   B   C   D   E   F   G   H(tok) I(prem) J(Δ) K(ν) L(stream)
    setup(ws, [26, 13, 11, 11, 11, 11, 11, 17,    12,     11,  11,  13])
    brand(ws, title, n)

    # --- Context block (top-left): the verbs read this set-once context. -----
    cc = 0
    for label, value, fkey in ctx:
        ws.write(5, cc, label, S["lab"])
        ws.write(5, cc + 1, value, S[fkey])
        cc += 3

    # --- LIVE strip (right of the context row): SUBSCRIBE / SERIES pulse. -----
    #     Sits on rows 5.. at cols K/L, above the ladder (row 8+) — no collision.
    if live:
        ws.write(5, VEGA_COL, "LIVE PULSE", S["labr"])
        lr = 5
        for label, f in live:
            ws.write(lr, VEGA_COL, label, S["lab"])
            wf(ws, lr, STREAM_COL, f, "val")
            lr += 1

    # The headline token cell of the FIRST section's FIRST row drives the
    # featured full-spill block below the ladder.
    headline = None

    r = 8
    for block in sections:
        sec(ws, r, block["title"], n, coral=block.get("coral", False))
        r += 1
        # column header rail (column-stable across every desk)
        rail = ["STRUCTURE", "PRODUCT", "TERMS →", "", "", "", "",
                "INSTRUMENT TOKEN", "PREMIUM", "Δ", "ν", "STREAM"]
        for c, t in enumerate(rail):
            ws.write(r, c, t, S["ull"] if c == 0 else S["ul"])
        r += 1
        for s in block["rows"]:
            r, tok = _emit_structure(ws, r, s, allow_wall)
            if headline is None and tok is not None:
                headline = (s["label"], tok)
        r += 1

    if notes:
        for text, coral in notes:
            ws.set_row(r, 26)
            ws.merge_range(r, 0, r, n, text, S["noteC"] if coral else S["note"])
            r += 1

    if headline is not None:
        r = _full_risk_block(ws, r + 1, n, headline)

    if extra is not None:
        r = extra(ws, r + 1, n)

    stamp(ws, r + 1, n)
    return ws


def _excel_str(v):
    """Render a Python value as an Excel formula string literal."""
    if isinstance(v, str):
        return '"' + v.replace('"', '""') + '"'
    return str(v)


def _emit_structure(ws, r, s, allow_wall):
    """Emit one scenario row: label | product | terms-matrix | token | scalar ladder.

    Returns `(next_row, token_cell_A1 | None)`. The token sits at the FIXED column
    H (TOK_COL), past the widest C..G terms matrix, so the strategy `legs` payload
    cells are NEVER overwritten by the token / verb formulas (the old collision).
    The verbs are NON-SPILLING scalars (INDEX over the dynamic array) so adjacent
    rows/columns never overlap; the full PRICE/RFQ spills are showcased once per
    sheet in the FULL RISK / TERM SHEET block.

    Enforces the capability matrix (HARD RULE 3): a cross-asset (equity/commodity/
    crypto) underlier with a non-leaf product is refused at BUILD time — UNLESS it
    is the single labelled capability-wall demo cell (s["wall"] and allow_wall).
    PERPETUAL is the one TENORLESS arm: it appends NO tenor argument (the shaper
    errors "PERPETUAL takes no tenor"), and its notional rides as a ("notional", …)
    term so the positional slot can never shift into the tenor argument.
    """
    label = s["label"]
    product = s["product"]
    underlier = s.get("underlier")
    underlier_cell = s.get("underlier_cell")  # an A1 ref string (e.g. "$B$6")
    is_wall = s.get("wall", False)
    is_perpetual = product.upper() == "PERPETUAL"

    # ---- Capability-matrix guard (enforced in ONE place) -------------------
    if underlier is not None and _is_cross_asset(underlier) and product.upper() not in LEAF_ARMS:
        if not (is_wall and allow_wall):
            raise ValueError(
                f"capability-matrix violation: {_leaf_class(underlier)} underlier "
                f"{underlier!r} cannot use {product!r} (leaf arms only: "
                f"{sorted(LEAF_ARMS)}). Mark it as the labelled capability-wall cell "
                f"to demo the typed build-time error."
            )

    ws.set_row(r, 17)
    ws.write(r, 0, label, S["pair"])
    ws.write(r, 1, product, S["inl"])
    prod_cell = xlsxwriter.utility.xl_rowcol_to_cell(r, 1)

    # ---- Terms matrix (HARD RULE 1): adjacent key/value cells -> a range ----
    # PERPETUAL carries its notional as a term (never a positional arg — see below).
    terms = list(s.get("terms", []))
    notional = s.get("notional")
    if is_perpetual and notional is not None:
        terms = terms + [("notional", notional)]
        notional = None
    if terms:
        rng, r_last = write_terms(ws, r, 2, terms)
    else:
        rng, r_last = None, r

    # ---- The INSTRUMENT token at the FIXED column H, past the terms matrix --
    if underlier_cell is not None:
        u_arg = underlier_cell
    else:
        u_arg = _excel_str(underlier)
    tenor = None if is_perpetual else s.get("tenor")
    args = [u_arg, prod_cell, (rng if rng else '{"_",""}')]
    # PERPETUAL omits the tenor argument entirely (it is the one tenorless arm).
    if tenor is not None:
        args.append(_excel_str(tenor))
    if notional is not None:
        args.append(str(notional) if not isinstance(notional, str) else notional)
    token_f = "=CELNET.INSTRUMENT(" + ",".join(args) + ")"
    wf(ws, r, TOK_COL, token_f, "tok")
    tok_cell = xlsxwriter.utility.xl_rowcol_to_cell(r, TOK_COL)

    # ---- The compact, NON-SPILLING scalar ladder (INDEX over the array) -----
    #   PREMIUM = PRICE row 1 col 2 ; Δ = GREEKS row 1 (delta_spot) ;
    #   ν = GREEKS row 4 (vega) — verified against shaping.ts GREEK_ROWS.
    verbs = s.get("verbs", ["PRICE", "GREEKS"])
    if "PRICE" in verbs:
        wf(ws, r, PREM_COL, f"=INDEX(CELNET.PRICE({tok_cell}),1,2)", "cor")
    if "GREEKS" in verbs:
        wf(ws, r, DELTA_COL, f"=INDEX(CELNET.GREEKS({tok_cell}),1,2)", "num")
        wf(ws, r, VEGA_COL, f"=INDEX(CELNET.GREEKS({tok_cell}),4,2)", "num")
    if "SUBSCRIBE" in verbs:
        wf(ws, r, STREAM_COL, f"=CELNET.SUBSCRIBE({tok_cell})", "val")

    # ---- Advance past the taller of (label row, terms matrix), +1 gap row ---
    return max(r_last, r) + 2, tok_cell


# ===========================================================================
#  1. COVER & LEGEND  — the 30-second orientation + the live firm pulse
# ===========================================================================
def build_cover():
    N = 11
    ws = wb.add_worksheet("Cover & Legend")
    ws.set_tab_color(CORAL)
    setup(ws, [20, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13])
    brand(ws, "Cover — one live contract, every asset class", N)

    ws.set_row(5, 28)
    ws.merge_range(5, 0, 5, N, "One live contract. Every asset class. In the cell.", S["hero"])
    ws.set_row(6, 18)
    ws.merge_range(6, 0, 6, N,
                   "One polymorphic INSTRUMENT token prices an FX seagull, a coin-margined BTC vanilla "
                   "and a gold TARF from the same cell — then PRICE / GREEKS / RFQ / SUBSCRIBE on it.",
                   S["heroS"])

    # --- LIVE PULSE strip across the foot of the hero -----------------------
    ws.write(8, 0, "STATUS", S["lab"])
    wf(ws, 8, 1, "=CELNET.STATUS()", "val")
    ws.write(8, 6, "LIVE SPOT", S["labr"])
    for i, (lab, f) in enumerate([
        ("EURUSD", '=CELNET.SERIES("EURUSD","SPOT")'),
        ("XAUUSD", '=CELNET.SERIES("XAUUSD","SPOT")'),
        ("BTC/USD", '=CELNET.SERIES("BTC/USD","SPOT")'),
    ]):
        ws.write(8, 7 + i, lab, S["ul"])
    # second row: the streaming spot values (column-aligned under the labels)
    for i, (lab, f) in enumerate([
        ("EURUSD", '=CELNET.SERIES("EURUSD","SPOT")'),
        ("XAUUSD", '=CELNET.SERIES("XAUUSD","SPOT")'),
        ("BTC/USD", '=CELNET.SERIES("BTC/USD","SPOT")'),
    ]):
        wf(ws, 9, 7 + i, f, "cor")

    # --- (1) THE TRADING DAY — numbered narrative, hyperlinked to each sheet -
    sec(ws, 11, "The trading day", N)
    day = [
        ("07:30", "FX Majors", "Market & Vol", "morning marking ritual"),
        ("07:30", "FX Majors", "FX Majors", "vanilla / skew / structured / var-vol flow"),
        ("08:15", "EM / NDF", "FX EM & NDF", "non-deliverable hedging, fixing-source edge"),
        ("09:00", "Metals", "Metals", "FULL 24-arm matrix — metals are first-class"),
        ("09:45", "Equity", "Equity", "cost-of-carry leaves + the capability wall"),
        ("10:15", "Commodity", "Commodity", "Black-76 forward world, hedge-accounting footer"),
        ("10:45", "Crypto", "Crypto", "linear vs inverse-coin side-by-side"),
        ("11:30", "Cross-Asset RV", "Cross-Asset RV", "blended vanilla / carry legs"),
        ("12:00", "Firm roll-up", "Risk Cockpit", "one numeraire, server-aggregated cube"),
    ]
    r = 12
    for t, who, sheet, desc in day:
        ws.write(r, 0, t, S["lab"])
        ws.write_url(r, 1, f"internal:'{sheet}'!A1", S["link"], f"{who}")
        ws.merge_range(r, 2, r, N, desc, S["note"])
        r += 1

    # --- (2) THE PATTERN — the 4-step diagram + underlier-grammar cheat-strip
    pr = r + 1
    sec(ws, pr, "The pattern — build once, then ask the verbs", N, coral=True)
    pr += 1
    steps = [
        ("1", "INSTRUMENT(underlier, product, terms) → a token"),
        ("2", "PRICE(token) → premium + the full family spill"),
        ("3", "GREEKS(token) → the 13-Greek vector + convention footer"),
        ("4", "RFQ(token) / SUBSCRIBE(token) → two-way + live stream"),
    ]
    for num, desc in steps:
        ws.write(pr, 0, num, S["step"])
        ws.merge_range(pr, 1, pr, N, desc, S["val"])
        pr += 1
    pr += 1
    ws.write(pr, 0, "GRAMMAR", S["lab"])
    cheats = [
        ("FX", "EURUSD"), ("metal", "XAUUSD"), ("equity", "AAPL@XNAS:USD"),
        ("commodity", "BRENT@:USD"), ("crypto", "BTC/USD:inverse"),
    ]
    cc = 1
    for kind, ex in cheats:
        ws.write(pr, cc, kind, S["ul"])
        ws.write(pr + 1, cc, ex, S["val"])
        cc += 2
    pr += 3

    # --- (3) CAPABILITY MATRIX legend ---------------------------------------
    sec(ws, pr, "Capability matrix — the constraint shown as a feature", N)
    pr += 1
    legend = [
        ("FX & METAL", "all 24 arms — vanilla, skew, barrier, touch, digital, TARF, "
                       "accumulator, var/vol swap, NDF, …", False),
        ("EQUITY / COMMODITY / CRYPTO", "cost-of-carry leaves: VANILLA + PERPETUAL "
                                        "(no tenor) + FUTUREOPTION only", True),
        ("CRYPTO axis", "carries LINEAR | INVERSE_COIN settlement — nothing else has it", True),
        ("Error model", "a leaf exotic (e.g. equity BARRIER) errors at BUILD time with a typed "
                        "message — never a silent #N/A wall", False),
    ]
    for k, v, coral in legend:
        ws.write(pr, 0, k, S["pair"])
        ws.merge_range(pr, 2, pr, N, v, S["noteC"] if coral else S["note"])
        ws.set_row(pr, 24)
        pr += 1
    pr += 1

    # --- Convention-transparency callout ------------------------------------
    sec(ws, pr, "Convention transparency — every number self-documents", N, coral=True)
    pr += 1
    ws.set_row(pr, 40)
    ws.merge_range(pr, 0, pr, N,
                   "Every CELNET.* spill carries a footer row: the resolved delta/ATM/premium "
                   "convention + cut, the surface_version and the timestamp that produced it. A number "
                   "never appears in the grid without the convention behind it — the #1 cure for "
                   "FX-options mismarks, and the incumbents' single biggest opacity turned into our default.",
                   S["note"])
    pr += 2
    stamp(ws, pr, N)


# ===========================================================================
#  2. MARKET & VOL  — the morning marking ritual (extends the existing layout)
# ===========================================================================
def build_market_vol():
    N = 10
    m = wb.add_worksheet("Market & Vol")
    m.set_tab_color(CORAL)
    setup(m, [13, 11, 11, 11, 11, 11, 13, 12, 12, 13])
    brand(m, "Market & vol — the morning marking ritual", N)
    m.write(5, 0, "STATUS", S["lab"])
    wf(m, 5, 1, "=CELNET.STATUS()", "val")
    m.write(5, 3, "TRADE DATE", S["lab"])
    m.write_formula(5, 4, "=TODAY()", S["date"])

    # OBSERVABLES — the live 'read' bar (indigo)
    sec(m, 7, "Observables — EURUSD (live read)", N)
    for c, t in enumerate(["", "SPOT", "ATM 1M", "RR25 1M", "BF25 1M", "FWD 1M"]):
        m.write(8, c, t, S["ull"] if c == 0 else S["ul"])
    m.write(9, 0, "EURUSD", S["pair"])
    for c, f, k in [
        (1, '=CELNET.SERIES("EURUSD","SPOT")', "num"),
        (2, '=CELNET.SERIES("EURUSD","ATM","1M")', "pct"),
        (3, '=CELNET.SERIES("EURUSD","RR","1M",25)', "pct"),
        (4, '=CELNET.SERIES("EURUSD","BF","1M",25)', "pct"),
        (5, '=CELNET.SERIES("EURUSD","FWD","1M")', "num"),
    ]:
        wf(m, 9, c, f, k)

    # SMILE MARKING grid — the 'contribute' bar (coral): editable, re-spills
    sec(m, 11, "EURUSD smile marking — bump ATM / RR / BF and watch", N, coral=True)
    for c, t in enumerate(["TENOR", "MONTHS", "EXPIRY", "ATM", "RR25", "BF25",
                           "25ΔC", "25ΔP", "MODEL", "SURF_VER"]):
        m.write(12, c, t, S["ull"] if c == 0 else S["ul"])
    grid = [("1W", 0.25, 11.8, -0.30, 0.22, "VV"), ("1M", 1, 10.6, -0.45, 0.25, "VV"),
            ("2M", 2, 10.4, -0.50, 0.26, "VV"), ("3M", 3, 10.2, -0.55, 0.28, "VV"),
            ("6M", 6, 10.0, -0.62, 0.30, "SABR"), ("1Y", 12, 9.9, -0.70, 0.33, "SABR"),
            ("2Y", 24, 9.9, -0.78, 0.36, "SABR")]
    first = 13
    for i, (ten, mo, atm, rr, bf, model) in enumerate(grid):
        r = first + i
        r1 = r + 1
        m.set_row(r, 17)
        m.write(r, 0, ten, S["inl"])
        m.write(r, 1, mo, S["in"])
        m.write_formula(r, 2, f"=EDATE($E$6,B{r1})", S["cd"])
        m.write(r, 3, atm, S["inp"])
        m.write(r, 4, rr, S["inp"])
        m.write(r, 5, bf, S["inp"])
        m.write_formula(r, 6, f"=D{r1}+F{r1}+E{r1}/2", S["pct"])
        m.write_formula(r, 7, f"=D{r1}+F{r1}-E{r1}/2", S["pct"])
        m.write(r, 8, model, S["in"])
        wf(m, r, 9,
           f'=CELNET.MARKSURFACE("EURUSD",A{r1},I{r1},D{r1}/100,E{r1}/100,F{r1}/100,2*E{r1}/100,2*F{r1}/100)',
           "val")
    last = first + len(grid) - 1

    # vol term-structure line chart (re-renders on edit)
    ch = wb.add_chart({"type": "line"})
    ch.set_title({"name": "EURUSD vol term structure — by expiry",
                  "name_font": {"name": FONT, "size": 12, "color": "#FFFFFF"}})
    cats = f"='Market & Vol'!$C${first + 1}:$C${last + 1}"
    for col, name, color, w in [("D", "ATM", INKL, 2.75), ("G", "25Δ Call", CORAL, 2.25),
                                ("H", "25Δ Put", INDIGO, 2.25)]:
        ch.add_series({"name": name, "categories": cats,
                       "values": f"='Market & Vol'!${col}${first + 1}:${col}${last + 1}",
                       "line": {"color": color, "width": w},
                       "marker": {"type": "circle", "size": 5, "fill": {"color": color},
                                  "border": {"color": color}}})
    ch.set_x_axis({"date_axis": True, "num_format": "mmm-yy", "num_font": {"color": SECON, "size": 8},
                   "line": {"color": GRID}, "major_gridlines": {"visible": False}})
    ch.set_y_axis({"num_font": {"color": SECON, "size": 8}, "line": {"color": GRID},
                   "major_gridlines": {"visible": True, "line": {"color": GRID}}})
    ch.set_legend({"position": "bottom", "font": {"name": FONT, "size": 9, "color": INKL}})
    ch.set_size({"width": 600, "height": 320})
    ch.set_chartarea({"border": {"none": True}, "fill": {"color": CANVAS}})
    ch.set_plotarea({"fill": {"color": PANEL}, "border": {"color": GRID}})
    m.insert_chart(first, 11, ch, {"x_offset": 8, "y_offset": 2})

    # CALIBRATED SMILE spill + VV-vs-SABR compare + a manual MARK contribution.
    # Each SURFACE smile spills a delta-pillar header + vol row (≈6 cols wide for
    # a 5-point smile); anchor the SABR compare at col I so the VV spill (A..) has
    # clear columns to its right and the two never overlap horizontally.
    sec(m, last + 3, "Calibrated smile — SURFACE spills (VV vs SABR side-by-side)", N)
    wf(m, last + 4, 0, '=CELNET.SURFACE("EURUSD","1M","VV")', "val")
    wf(m, last + 4, 8, '=CELNET.SURFACE("EURUSD","1M","SABR")', "val")
    sec(m, last + 14, "Manual mark contribution — MARK (two-phase, commits via the task pane)",
        N, coral=True)
    wf(m, last + 15, 0, '=CELNET.MARK("EURUSD","3M","25dP",0.105,"VV","10:00 NY cut")', "val")
    stamp(m, last + 19, N)


# ===========================================================================
#  3..9  THE DESKS — all flow through desk_sheet(...)
# ===========================================================================
def build_fx_majors():
    desk_sheet(
        "FX Majors", GREEN, "FX majors — 07:30 the full 24-arm flow", 9,
        ctx=[("PAIR", "EURUSD", "inl"), ("NOTIONAL", 10000000, "inc")],
        sections=[
            {"title": "Vanilla & skew", "coral": False, "rows": [
                {"label": "Vanilla call 25Δ (corporate hedge)", "underlier": "EURUSD",
                 "product": "VANILLA", "tenor": "1M", "notional": "$E$6",
                 "terms": [("strike", "25dC"), ("callPut", "C")],
                 "verbs": ["PRICE", "GREEKS", "RFQ", "SUBSCRIBE"]},
                {"label": "Risk reversal 25Δ (trade the skew)", "underlier": "EURUSD",
                 "product": "RISKREVERSAL", "tenor": "3M", "notional": "$E$6",
                 "terms": [("legs", ["C", "25dC", "BUY"]), ("legs", ["P", "25dP", "SELL"])],
                 "verbs": ["PRICE", "GREEKS", "RFQ"]},
                {"label": "Straddle ATM (vol level)", "underlier": "EURUSD",
                 "product": "STRADDLE", "tenor": "1M", "notional": "$E$6",
                 "terms": [("legs", ["C", "ATM", "BUY"]), ("legs", ["P", "ATM", "BUY"])],
                 "verbs": ["PRICE", "GREEKS"]},
                {"label": "Strangle 25Δ (wings)", "underlier": "EURUSD",
                 "product": "STRANGLE", "tenor": "1M", "notional": "$E$6",
                 "terms": [("legs", ["C", "25dC", "BUY"]), ("legs", ["P", "25dP", "BUY"])],
                 "verbs": ["PRICE", "GREEKS"]},
            ]},
            {"title": "Structured flow — USDJPY book", "coral": True, "rows": [
                {"label": "Up-&-out call (KO barrier)", "underlier": "USDJPY",
                 "product": "BARRIER", "tenor": "3M", "notional": "$E$6",
                 "terms": [("strike", 158), ("callPut", "C"), ("barrier", 162),
                           ("kind", "KNOCK_OUT"), ("side", "UP")],
                 "verbs": ["PRICE", "GREEKS", "RFQ"]},
                {"label": "One-touch (rebate on touch)", "underlier": "USDJPY",
                 "product": "TOUCH", "tenor": "3M",
                 "terms": [("kind", "ONE_TOUCH"), ("barrier", 160), ("rebate", 100000)],
                 "verbs": ["PRICE", "GREEKS"]},
                {"label": "Digital call (cash-or-nothing)", "underlier": "USDJPY",
                 "product": "DIGITAL", "tenor": "1M",
                 "terms": [("strike", 158), ("callPut", "C"), ("payout", 100000)],
                 "verbs": ["PRICE", "GREEKS"]},
            ]},
            {"title": "Corporate programs — GBPUSD (absolute strikes)", "coral": False, "rows": [
                {"label": "TARF (target-redemption fwd)", "underlier": "GBPUSD",
                 "product": "TARF", "tenor": "12M", "notional": "$E$6",
                 "terms": [("callPut", "C"), ("strike", 1.27), ("target", 0.30),
                           ("leverage", 2), ("fixings", 12)],
                 "verbs": ["PRICE", "GREEKS"]},
                {"label": "Accumulator (pivot/barrier)", "underlier": "GBPUSD",
                 "product": "ACCUMULATOR", "tenor": "6M", "notional": "$E$6",
                 "terms": [("pivot", 1.25), ("barrier", 1.30), ("leverage", 2), ("fixings", 126)],
                 "verbs": ["PRICE", "GREEKS"]},
            ]},
            {"title": "Pure vol", "coral": True, "rows": [
                {"label": "Variance swap", "underlier": "EURUSD",
                 "product": "VARSWAP", "tenor": "3M",
                 "terms": [("strikeVol", 0.075)],
                 "verbs": ["PRICE"]},
                {"label": "Volatility swap", "underlier": "EURUSD",
                 "product": "VOLSWAP", "tenor": "3M",
                 "terms": [("strikeVol", 0.072)],
                 "verbs": ["PRICE"]},
            ]},
        ],
        live=[("EURUSD RR3M", '=CELNET.SERIES("EURUSD","RR","3M",25)'),
              ("EURUSD ATM3M", '=CELNET.SERIES("EURUSD","ATM","3M")')],
        notes=[("Murex one-screen term sheet: the editable terms matrix sits next to its premium and "
                "Greeks — bump a strike or a barrier and the whole row re-spills. 24 arms on one "
                "desk; equity / crypto can touch only three of them.", False)],
    )


def build_fx_em():
    desk_sheet(
        "FX EM & NDF", INDIGO, "FX EM & NDF — 08:15 non-deliverable hedging", 9,
        ctx=[("USDBRL ntl", 5000000, "inc"), ("USDKRW ntl", 5000000, "inc")],
        sections=[
            {"title": "NDF ladder — fixing source is the story", "coral": False, "rows": [
                {"label": "USDBRL NDF (PTAX-fixed)", "underlier": "USDBRL",
                 "product": "NDF", "tenor": "3M", "notional": "$B$6",
                 "terms": [("rate", 5.05), ("fixing", "PTAX"), ("settlementCcy", "USD")],
                 "verbs": ["PRICE", "GREEKS", "RFQ"]},
                {"label": "USDKRW NDF (KFTC-fixed)", "underlier": "USDKRW",
                 "product": "NDF", "tenor": "3M", "notional": "$E$6",
                 "terms": [("rate", 1330), ("fixing", "KFTC18"), ("settlementCcy", "USD")],
                 "verbs": ["PRICE", "GREEKS", "RFQ"]},
            ]},
            {"title": "Vol overlay — wider EM skew", "coral": True, "rows": [
                {"label": "USDKRW vanilla ATM (vol view)", "underlier": "USDKRW",
                 "product": "VANILLA", "tenor": "1M", "notional": "$E$6",
                 "terms": [("strike", "ATM"), ("callPut", "C")],
                 "verbs": ["PRICE", "GREEKS", "SUBSCRIBE"]},
            ]},
        ],
        live=[("USDBRL FWD3M", '=CELNET.SERIES("USDBRL","FWD","3M")'),
              ("USDKRW FWD3M", '=CELNET.SERIES("USDKRW","FWD","3M")')],
        notes=[("The FORWARD / SWAP / NDF spill carries a Monte-Carlo std_error row before the Greeks "
                "— vertical room is reserved so it never collides. The convention footer surfaces "
                "the fixing source (PTAX / KFTC18) and the business-day rule: the EM-settlement-"
                "correctness edge, on every row.", False)],
    )


def _metals_surface_block(ws, r, n):
    """SURFACE + MARKSURFACE on XAUUSD — the full FX-grade smile on a metal.

    Both verbs spill a ≈6-col delta-pillar smile (header + vol + footer); stack
    them vertically with clear rows so the two roomy spills never overlap.
    """
    sec(ws, r, "Surface marking — full FX-grade smile on a metal (SABR)", n, coral=True)
    ws.write(r + 1, 0, "SURFACE 3M ↓", S["lab"])
    wf(ws, r + 2, 0, '=CELNET.SURFACE("XAUUSD","3M","SABR")', "val")
    ws.write(r + 6, 0, "MARKSURFACE 3M ↓", S["lab"])
    wf(ws, r + 7, 0, '=CELNET.MARKSURFACE("XAUUSD","3M","SABR",0.145,-0.012,0.004,0,0)', "val")
    return r + 11


def build_metals():
    desk_sheet(
        "Metals", GOLD, "Metals — 09:00 metals are FIRST-CLASS (full 24 arms)", 9,
        ctx=[("XAUUSD ntl", 5000, "inc"), ("XAGUSD ntl", 250000, "inc")],
        sections=[
            {"title": "Structures — exotics ARE legal on metal", "coral": False, "rows": [
                {"label": "XAU zero-cost collar (seagull)", "underlier": "XAUUSD",
                 "product": "SEAGULL", "tenor": "6M", "notional": "$B$6",
                 "terms": [("legs", ["P", "25dP", "BUY"]), ("legs", ["C", "25dC", "SELL"]),
                           ("legs", ["C", "10dC", "BUY"])],
                 "verbs": ["PRICE", "GREEKS", "RFQ"]},
                {"label": "XAG one-touch (silver squeeze)", "underlier": "XAGUSD",
                 "product": "TOUCH", "tenor": "3M", "notional": "$E$6",
                 "terms": [("kind", "ONE_TOUCH"), ("barrier", 35), ("rebate", 50000)],
                 "verbs": ["PRICE", "GREEKS"]},
                {"label": "XAU TARF (exotics on metal)", "underlier": "XAUUSD",
                 "product": "TARF", "tenor": "12M", "notional": "$B$6",
                 "terms": [("callPut", "P"), ("strike", 2300), ("target", 100),
                           ("leverage", 2), ("fixings", 12)],
                 "verbs": ["PRICE", "GREEKS"]},
            ]},
        ],
        live=[("XAUUSD RR3M", '=CELNET.SERIES("XAUUSD","RR","3M",25)'),
              ("XAUUSD SPOT", '=CELNET.SERIES("XAUUSD","SPOT")')],
        notes=[("Full FX-grade smile on a metal — the next three desks (equity / commodity / "
                "crypto) are cost-of-carry leaves and cannot do this. Every incumbent that treats "
                "metals as a degraded commodity leaf is beaten here.", True)],
        extra=_metals_surface_block,
    )


def build_equity():
    desk_sheet(
        "Equity", EQBLUE, "Equity — 09:45 cost-of-carry leaves + the capability wall", 9,
        ctx=[("UNDERLIER", "AAPL@XNAS:USD", "inl"), ("SHARES", 1000, "inc")],
        sections=[
            {"title": "Leaves — generalized-BSM dividend-yield carry", "coral": False, "rows": [
                {"label": "AAPL overwriting call (vanilla)", "underlier": "AAPL@XNAS:USD",
                 "product": "VANILLA", "tenor": "1M", "notional": 1000,
                 "terms": [("strike", 230), ("callPut", "C")],
                 "verbs": ["PRICE", "GREEKS", "SUBSCRIBE"]},
                {"label": "AAPL perpetual call (NO tenor)", "underlier": "AAPL@XNAS:USD",
                 "product": "PERPETUAL", "tenor": None, "notional": 1000,
                 "terms": [("strike", 250), ("callPut", "C")],
                 "verbs": ["PRICE", "GREEKS"]},
                {"label": "ES future-option hedge", "underlier": "ES@:USD",
                 "product": "FUTUREOPTION", "tenor": "3M", "notional": 1,
                 "terms": [("strike", 5600), ("callPut", "C"),
                           ("futureSymbol", "ESU6@XCME"), ("futureExpiry", 0.27)],
                 "verbs": ["PRICE", "GREEKS"]},
            ]},
            {"title": "THE CAPABILITY WALL — a deliberate demonstration", "coral": True, "rows": [
                {"label": "Equity BARRIER (INTENTIONAL ERROR)", "underlier": "AAPL@XNAS:USD",
                 "product": "BARRIER", "tenor": "3M", "notional": 1000,
                 "terms": [("strike", 230), ("callPut", "C"), ("barrier", 260),
                           ("kind", "KNOCK_OUT"), ("side", "UP")],
                 "verbs": ["PRICE"], "wall": True},
            ]},
        ],
        live=[("AAPL SPOT", '=CELNET.SERIES("AAPL@XNAS:USD","SPOT")')],
        notes=[("CAPABILITY WALL (deliberate, not a bug): equity is a cost-of-carry leaf — only "
                "VANILLA / PERPETUAL / FUTUREOPTION are priceable. The BARRIER above errors at BUILD "
                "time with a typed, legible message naming the asset class — NOT a silent #N/A. "
                "This is the direct answer to Bloomberg add-ins' #N/A storms.", True),
               ("FUTUREOPTION needs futureSymbol + futureExpiry (year fraction ≥ the option "
                "expiry); PERPETUAL takes no tenor argument at all.", False)],
        allow_wall=True,
    )


def build_commodity():
    desk_sheet(
        "Commodity", COMMBR, "Commodity — 10:15 the Black-76 forward world", 9,
        ctx=[("UNDERLIER", "BRENT@:USD", "inl"), ("BARRELS", 10000, "inc")],
        sections=[
            {"title": "Leaves — forward-based vanilla + future-option", "coral": False, "rows": [
                {"label": "Brent call (airline hedge)", "underlier": "BRENT@:USD",
                 "product": "VANILLA", "tenor": "3M", "notional": 10000,
                 "terms": [("strike", 85), ("callPut", "C")],
                 "verbs": ["PRICE", "GREEKS", "RFQ", "SUBSCRIBE"]},
                {"label": "WTI future-option put", "underlier": "WTI@:USD",
                 "product": "FUTUREOPTION", "tenor": "3M", "notional": 1,
                 "terms": [("strike", 78), ("callPut", "P"),
                           ("futureSymbol", "CLZ6@XNYM"), ("futureExpiry", 0.50)],
                 "verbs": ["PRICE", "GREEKS"]},
            ]},
        ],
        live=[("BRENT SPOT", '=CELNET.SERIES("BRENT@:USD","SPOT")')],
        notes=[("Commodity = leaf (VANILLA / PERPETUAL / FUTUREOPTION only), exactly like equity. The "
                "convention footer surfaces the day-count and settlement fields the airline's "
                "hedge-accounting team ties out against — carry / convenience-yield transparency "
                "on every spill.", False)],
    )


def build_crypto():
    desk_sheet(
        "Crypto", INDIGO, "Crypto — 10:45 LINEAR vs INVERSE-COIN side-by-side", 9,
        ctx=[("STRIKE", 70000, "inc"), ("BTC ntl", 10, "inc")],
        sections=[
            {"title": "Same strike, two settlements — read the payoff difference", "coral": True,
             "rows": [
                {"label": "BTC linear (USD-margined) call", "underlier": "BTC/USD:linear",
                 "product": "VANILLA", "tenor": "1M", "notional": 10,
                 "terms": [("strike", 70000), ("callPut", "C")],
                 "verbs": ["PRICE", "GREEKS"]},
                {"label": "BTC inverse (coin-margined 1/S_T) call", "underlier": "BTC/USD:inverse",
                 "product": "VANILLA", "tenor": "1M", "notional": 10,
                 "terms": [("strike", 70000), ("callPut", "C")],
                 "verbs": ["PRICE", "GREEKS", "SUBSCRIBE"]},
            ]},
            {"title": "Perpetual sleeve (NO tenor)", "coral": False, "rows": [
                {"label": "ETH perpetual call", "underlier": "ETH/USD:linear",
                 "product": "PERPETUAL", "tenor": None, "notional": 100,
                 "terms": [("strike", 4000), ("callPut", "C")],
                 "verbs": ["PRICE", "GREEKS"]},
            ]},
        ],
        live=[("BTC/USD SPOT", '=CELNET.SERIES("BTC/USD","SPOT")')],
        notes=[("The settlement-axis showpiece: identical strike, two INSTRUMENT tokens. The inverse "
                "leg prices the coin-margined 1/S_T payoff — the demo that sells inverse-payoff "
                "correctness. No incumbent's add-in structures multi-asset.", True),
               ("Known-coin rule: BTC / ETH / SOL… select crypto even in 3/3 form; any other coin "
                "uses a non-3-letter leg (FOO/USDT) or an explicit :linear / :inverse suffix.", False)],
    )


def build_cross_asset():
    desk_sheet(
        "Cross-Asset RV", VIOLET, "Cross-asset RV — 11:30 the honest envelope", 9,
        ctx=[("NUMERAIRE", "USD", "inl"), ("FX ntl", 10000000, "inc")],
        sections=[
            {"title": "(1) Gold vs USD vol — net vega RV", "coral": False, "rows": [
                {"label": "XAU straddle (long vol)", "underlier": "XAUUSD",
                 "product": "STRADDLE", "tenor": "3M", "notional": 5000,
                 "terms": [("legs", ["C", "ATM", "BUY"]), ("legs", ["P", "ATM", "BUY"])],
                 "verbs": ["PRICE", "GREEKS"]},
                {"label": "EURUSD straddle (short vol)", "underlier": "EURUSD",
                 "product": "STRADDLE", "tenor": "3M", "notional": "$E$6",
                 "terms": [("legs", ["C", "ATM", "SELL"]), ("legs", ["P", "ATM", "SELL"])],
                 "verbs": ["PRICE", "GREEKS"]},
            ]},
            {"title": "(2) BTC vs tech-beta — both legs vanilla (the allowed envelope)",
             "coral": True, "rows": [
                {"label": "BTC linear 25Δ call", "underlier": "BTC/USD:linear",
                 "product": "VANILLA", "tenor": "1M", "notional": 10,
                 "terms": [("strike", "25dC"), ("callPut", "C")],
                 "verbs": ["PRICE", "GREEKS"]},
                {"label": "AAPL 25Δ call", "underlier": "AAPL@XNAS:USD",
                 "product": "VANILLA", "tenor": "1M", "notional": 1000,
                 "terms": [("strike", "25dC"), ("callPut", "C")],
                 "verbs": ["PRICE", "GREEKS"]},
            ]},
            {"title": "(3) Treasury overlay — rolls into the firm cube →", "coral": False,
             "rows": [
                {"label": "GBPUSD TARF", "underlier": "GBPUSD",
                 "product": "TARF", "tenor": "12M", "notional": "$E$6",
                 "terms": [("callPut", "C"), ("strike", 1.27), ("target", 0.30),
                           ("leverage", 2), ("fixings", 12)],
                 "verbs": ["PRICE"]},
                {"label": "Brent call", "underlier": "BRENT@:USD",
                 "product": "VANILLA", "tenor": "3M", "notional": 10000,
                 "terms": [("strike", 85), ("callPut", "C")],
                 "verbs": ["PRICE"]},
                {"label": "XAU seagull collar", "underlier": "XAUUSD",
                 "product": "SEAGULL", "tenor": "6M", "notional": 5000,
                 "terms": [("legs", ["P", "25dP", "BUY"]), ("legs", ["C", "25dC", "SELL"]),
                           ("legs", ["C", "10dC", "BUY"])],
                 "verbs": ["PRICE"]},
                {"label": "BTC linear vanilla sleeve", "underlier": "BTC/USD:linear",
                 "product": "VANILLA", "tenor": "1M", "notional": 10,
                 "terms": [("strike", 70000), ("callPut", "C")],
                 "verbs": ["PRICE"]},
            ]},
        ],
        live=[("XAUUSD ATM3M", '=CELNET.SERIES("XAUUSD","ATM","3M")'),
              ("BTC/USD SPOT", '=CELNET.SERIES("BTC/USD","SPOT")')],
        notes=[("Honest cross-asset envelope: FX/metals legs can be ANY arm, but equity/commodity/"
                "crypto legs are vanilla / perpetual / future-option only. Cross-asset blends combine "
                "vanilla & carry legs on the leaves; exotics stay FX/metals-only — the constraint "
                "is a feature: one consistent delta convention across BTC and AAPL.", True)],
    )


# ===========================================================================
#  10. RISK COCKPIT  — the 12:00 head-of-desk firm roll-up (bespoke)
# ===========================================================================
def build_risk_cockpit():
    N = 9
    rk = wb.add_worksheet("Risk Cockpit")
    rk.set_tab_color(VIOLET)
    setup(rk, [18, 14, 14, 14, 14, 14, 14, 14, 14, 14])
    brand(rk, "Risk cockpit — 12:00 the firm roll-up", N)
    rk.write(5, 0, "STATUS", S["lab"])
    wf(rk, 5, 1, "=CELNET.STATUS()", "val")
    rk.write(5, 4, "NUMERAIRE", S["lab"])
    rk.write(5, 5, "USD", S["in"])  # $F$6 is the numeraire — but blueprint uses $B$7; align below

    # --- CROSS-RATES table (editable [ccy,rate] range) — auditable roll-up ---
    sec(rk, 7, "Cross-rates — editable [ccy, rate] (numeraire units per 1 ccy)", N, coral=True)
    rk.write(8, 0, "NUMERAIRE", S["lab"])
    rk.write(8, 1, "USD", S["in"])  # $B$9 — the numeraire cell the verbs reference
    for c, t in enumerate(["CCY", "RATE"]):
        rk.write(9, c, t, S["ull"] if c == 0 else S["ul"])
    rates = [("EUR", 1.09), ("GBP", 1.27), ("JPY", 0.0064), ("BRL", 0.196),
             ("KRW", 0.00075), ("XAU", 2320.0), ("XAG", 30.5), ("BTC", 68000.0)]
    r0 = 10
    for i, (ccy, rate) in enumerate(rates):
        rr = r0 + i
        rk.write(rr, 0, ccy, S["inl"])
        rk.write(rr, 1, rate, S["inp"])
    r1 = r0 + len(rates) - 1
    NUM = "$B$9"
    RATES = f"$A${r0 + 1}:$B${r1 + 1}"

    # --- POSITIONS (entitled book, deny-by-default) -------------------------
    pr = r1 + 2
    sec(rk, pr, "Positions — the entitled book (deny-by-default entitlements)", N)
    wf(rk, pr + 1, 0, '=CELNET.POSITIONS("FIRM")', "val")
    rk.merge_range(pr + 1, 4, pr + 1, N,
                   "Deny-by-default: the spill lists only the entitled book; every client asserts an "
                   "explicit grant. No silent over-disclosure.", S["note"])

    # --- FIRM RISK cube (diverging heat) ------------------------------------
    fr = pr + 8
    sec(rk, fr, "Firm risk cube — RISK(FIRM) in one numeraire (diverging heat)", N, coral=True)
    wf(rk, fr + 1, 0, f'=CELNET.RISK("FIRM",{NUM},{RATES})', "val")
    # Diverging 3-colour heat on the spilled delta/vega numeric region (generous).
    heat_first = fr + 2
    heat_last = fr + 14
    rk.conditional_format(heat_first, 2, heat_last, 6, {
        "type": "3_color_scale",
        "min_color": RAG_R, "mid_color": PANEL, "max_color": RAG_G,
        "min_type": "min", "mid_type": "percentile", "mid_value": 50, "max_type": "max",
    })

    # --- BY-DESK drill + scoped TRADER drill --------------------------------
    dr = fr + 16
    sec(rk, dr, "Drill-downs — by desk, and a scoped trader view", N)
    rk.write(dr + 1, 0, "BY DESK", S["lab"])
    wf(rk, dr + 2, 0, f'=CELNET.RISK("DESK",{NUM},{RATES})', "val")
    rk.write(dr + 1, 5, "TRADER @ DESK:99", S["lab"])
    wf(rk, dr + 2, 5, f'=CELNET.RISK("TRADER",{NUM},{RATES},"DESK:99")', "val")

    # --- LIMITS (RAG conditional formatting) --------------------------------
    lr = dr + 12
    sec(rk, lr, "Limits — utilization + RAG (firm + the EM NDF desk)", N, coral=True)
    rk.write(lr + 1, 0, "FIRM", S["lab"])
    wf(rk, lr + 2, 0, f'=CELNET.LIMITS("FIRM",{NUM},{RATES})', "val")
    rk.write(lr + 1, 5, "DESK:99", S["lab"])
    wf(rk, lr + 2, 5, f'=CELNET.LIMITS("DESK:99",{NUM},{RATES})', "val")
    # RAG colour scale across the spilled utilization region (green->amber->red).
    lim_first = lr + 3
    lim_last = lr + 13
    rk.conditional_format(lim_first, 1, lim_last, 6, {
        "type": "3_color_scale",
        "min_color": RAG_G, "mid_color": RAG_A, "max_color": RAG_R,
        "min_type": "num", "min_value": 0,
        "mid_type": "num", "mid_value": 0.75,
        "max_type": "num", "max_value": 1.0,
    })

    cr = lim_last + 2
    rk.set_row(cr, 40)
    rk.merge_range(cr, 0, cr, N,
                   "One engine, bit-identical across GUI / SDK / CLI / Excel. The spreadsheet is a thin "
                   "view; the FIRM → DESK → BOOK → TRADER → PAIR hierarchy is rolled "
                   "up SERVER-SIDE and spilled — incumbents push raw rows and make Excel aggregate, "
                   "or lock the roll-up in a separate risk app. Gold ounces, BTC coins and EUR notionals "
                   "all land in USD via the editable cross-rates above.", S["note"])
    stamp(rk, cr + 2, N)


# ===========================================================================
#  Build the workbook in trading-day tab order, then close.
# ===========================================================================
build_cover()
build_market_vol()
build_fx_majors()
build_fx_em()
build_metals()
build_equity()
build_commodity()
build_crypto()
build_cross_asset()
build_risk_cockpit()

wb.close()
print("WROTE", OUT)
