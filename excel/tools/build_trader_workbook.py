#!/usr/bin/env python3
"""Celnet trader workbook — Celer Technologies dark theme, live add-in model.

Dark navy canvas + coral/indigo accents + the pinwheel + Anaheim (Celer brand kit).
Uses ONLY the 13 registered custom functions (exotics via INSTRUMENT -> PRICE/GREEKS),
every CELNET.* formula written with the _xlfn. prefix Excel requires for add-in
functions in an externally-generated file. Editable inputs reprice against the edge.

    /tmp/celnet-xlsx-venv/bin/python excel/tools/build_trader_workbook.py [out.xlsx]
"""
import os
import sys
import xlsxwriter

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/Desktop/Celnet-Trader.xlsx")
LOGO = "/tmp/celer-logo.png"

# ---- Celer dark palette (memory: celer-brand-kit) --------------------------
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
FONT, MONO = "Anaheim", "Menlo"

wb = xlsxwriter.Workbook(OUT, {"nan_inf_to_errors": True})
wb.set_calc_mode("auto")

def cf(f):
    # Office.js add-in custom functions resolve as plain NAMESPACE.NAME (no _xlfn.
    # prefix — that's only for built-ins). Empirically confirmed: a typed
    # =CELNET.STATUS() resolves while =_xlfn.CELNET.STATUS() #NAME?s.
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
    "ul": fmt(bold=True, font_size=8, font_color=SECON, align="center", bottom=1, bottom_color=GRID),
    "ull": fmt(bold=True, font_size=8, font_color=SECON, bottom=1, bottom_color=GRID),
    "pair": fmt(bold=True, font_size=10, font_color="#FFFFFF"),
    "val": fmt(font_name=MONO, font_size=10, font_color=INKL),
    "num": fmt(font_name=MONO, font_size=10, font_color=INKL, num_format="0.0000", align="center"),
    "pct": fmt(font_name=MONO, font_size=10, font_color=INKL, num_format="0.00", align="center"),
    "cor": fmt(font_name=MONO, font_size=10, font_color=CORAL, num_format="0.0000", align="center"),
    "axe": fmt(bold=True, font_size=9, font_color=CORAL, align="center"),
    "cd": fmt(font_name=MONO, font_size=9, font_color=SECON, num_format="dd-mmm-yy", align="center"),
    "in": fmt(font_size=10, font_color="#FFFFFF", bg_color=INPUT, align="center", bottom=2, bottom_color=CORAL),
    "inl": fmt(font_size=10, font_color="#FFFFFF", bg_color=INPUT, bottom=2, bottom_color=CORAL),
    "inp": fmt(font_size=10, font_color="#FFFFFF", bg_color=INPUT, num_format="0.00", align="center", bottom=2, bottom_color=CORAL),
    "inc": fmt(font_size=10, font_color="#FFFFFF", bg_color=INPUT, num_format="#,##0", align="center", bottom=2, bottom_color=CORAL),
    "date": fmt(font_size=10, font_color="#FFFFFF", bg_color=INPUT, num_format="dd-mmm-yyyy", align="center", bottom=2, bottom_color=CORAL),
}

def setup(ws, widths):
    ws.hide_gridlines(2)
    ws.set_paper(9); ws.set_landscape()
    for c, w in enumerate(widths):
        ws.set_column(c, c, w, S["canvas"])
    ws.set_column(len(widths), 40, 11, S["canvas"])  # extend the dark canvas right

def brand(ws, title, n):
    ws.set_row(0, 6); ws.merge_range(0, 0, 0, n, "", S["coral"])
    ws.set_row(1, 46); ws.merge_range(1, 0, 1, n, "", S["band"])
    if os.path.exists(LOGO):
        ws.insert_image(1, 0, LOGO, {"x_offset": 14, "y_offset": 7, "x_scale": 0.165, "y_scale": 0.165})
    ws.write_rich_string(1, 1, S["word"], "Celnet  ", S["wsub"], "/  CELER TECHNOLOGIES")
    ws.set_row(2, 4); ws.merge_range(2, 0, 2, n, "", S["band"])
    ws.set_row(3, 20); ws.merge_range(3, 0, 3, n, "   " + title.upper(), S["title"])
    ws.freeze_panes(4, 0)

def sec(ws, r, text, n, coral=False):
    ws.set_row(r, 18); ws.merge_range(r, 0, r, n, "  " + text.upper(), S["secC"] if coral else S["secI"])

def stamp(ws, r, n):
    ws.set_row(r, 16)
    ws.merge_range(r, 0, r, n, "CELNET v1.0-RC    ws://127.0.0.1:8081    GUI · SDK · CLI · EXCEL PARITY   ", S["stamp"])

def wf(ws, r, c, f, key):
    ws.write_formula(r, c, cf(f), S[key])

# ============================================================== Market & Vol
N = 10
m = wb.add_worksheet("Market & Vol"); m.set_tab_color(CORAL)
setup(m, [13, 11, 11, 11, 11, 11, 13, 12, 12, 13])
brand(m, "Market & vol", N)
m.write(5, 0, "STATUS", S["lab"]); wf(m, 5, 1, '=CELNET.STATUS()', "val")
m.write(5, 3, "TRADE DATE", S["lab"]); m.write_formula(5, 4, "=TODAY()", S["date"])
sec(m, 7, "Observables — EURUSD", N)
for c, t in enumerate(["", "SPOT", "ATM 1M", "RR25 1M", "BF25 1M", "FWD 1M"]):
    m.write(8, c, t, S["ull"] if c == 0 else S["ul"])
m.write(9, 0, "EURUSD", S["pair"])
for c, f, k in [(1, '=CELNET.SERIES("EURUSD","SPOT")', "num"), (2, '=CELNET.SERIES("EURUSD","ATM","1M")', "pct"),
                (3, '=CELNET.SERIES("EURUSD","RR","1M",25)', "pct"), (4, '=CELNET.SERIES("EURUSD","BF","1M",25)', "pct"),
                (5, '=CELNET.SERIES("EURUSD","FWD","1M")', "num")]:
    wf(m, 9, c, f, k)
sec(m, 11, "EURUSD smile marking", N, coral=True)
for c, t in enumerate(["TENOR", "MONTHS", "EXPIRY", "ATM", "RR25", "BF25", "25ΔC", "25ΔP", "MODEL", "SURF_VER"]):
    m.write(12, c, t, S["ull"] if c == 0 else S["ul"])
grid = [("1W", 0.25, 11.8, -0.30, 0.22, "VV"), ("1M", 1, 10.6, -0.45, 0.25, "VV"),
        ("2M", 2, 10.4, -0.50, 0.26, "VV"), ("3M", 3, 10.2, -0.55, 0.28, "VV"),
        ("6M", 6, 10.0, -0.62, 0.30, "SABR"), ("1Y", 12, 9.9, -0.70, 0.33, "SABR"),
        ("2Y", 24, 9.9, -0.78, 0.36, "SABR")]
first = 13
for i, (ten, mo, atm, rr, bf, model) in enumerate(grid):
    r = first + i; r1 = r + 1; m.set_row(r, 17)
    m.write(r, 0, ten, S["inl"]); m.write(r, 1, mo, S["in"])
    m.write_formula(r, 2, f"=EDATE($E$6,B{r1})", S["cd"])
    m.write(r, 3, atm, S["inp"]); m.write(r, 4, rr, S["inp"]); m.write(r, 5, bf, S["inp"])
    m.write_formula(r, 6, f"=D{r1}+F{r1}+E{r1}/2", S["pct"])
    m.write_formula(r, 7, f"=D{r1}+F{r1}-E{r1}/2", S["pct"])
    m.write(r, 8, model, S["in"])
    wf(m, r, 9, f'=CELNET.MARKSURFACE("EURUSD",A{r1},I{r1},D{r1}/100,E{r1}/100,F{r1}/100,2*E{r1}/100,2*F{r1}/100)', "val")
last = first + len(grid) - 1
ch = wb.add_chart({"type": "line"})
ch.set_title({"name": "EURUSD vol term structure — by expiry",
              "name_font": {"name": FONT, "size": 12, "color": "#FFFFFF"}})
cats = f"='Market & Vol'!$C${first+1}:$C${last+1}"
for col, name, color, w in [("D", "ATM", INKL, 2.75), ("G", "25Δ Call", CORAL, 2.25), ("H", "25Δ Put", INDIGO, 2.25)]:
    ch.add_series({"name": name, "categories": cats,
        "values": f"='Market & Vol'!${col}${first+1}:${col}${last+1}",
        "line": {"color": color, "width": w}, "marker": {"type": "circle", "size": 5, "fill": {"color": color}, "border": {"color": color}}})
ch.set_x_axis({"date_axis": True, "num_format": "mmm-yy", "num_font": {"color": SECON, "size": 8},
               "line": {"color": GRID}, "major_gridlines": {"visible": False}})
ch.set_y_axis({"num_font": {"color": SECON, "size": 8}, "line": {"color": GRID},
               "major_gridlines": {"visible": True, "line": {"color": GRID}}})
ch.set_legend({"position": "bottom", "font": {"name": FONT, "size": 9, "color": INKL}})
ch.set_size({"width": 600, "height": 320})
ch.set_chartarea({"border": {"none": True}, "fill": {"color": CANVAS}})
ch.set_plotarea({"fill": {"color": PANEL}, "border": {"color": GRID}})
m.insert_chart(first, 11, ch, {"x_offset": 8, "y_offset": 2})
sec(m, last + 3, "Calibrated smile — CELNET.SURFACE (spills)", N)
wf(m, last + 4, 0, '=CELNET.SURFACE("EURUSD","1M","VV")', "val")
stamp(m, last + 13, N)

# ===================================================================== Axes
ax = wb.add_worksheet("Axes"); ax.set_tab_color(INDIGO)
setup(ax, [11, 11, 11, 11, 4, 11, 9, 9, 9, 15, 13])
brand(ax, "Axes — multi-pair", N)
ax.write(5, 0, "RFQ SIZE (mm)", S["lab"]); ax.write(5, 1, 10, S["inc"])
ax.write(5, 5, "VOL TENOR", S["lab"]); ax.write(5, 6, "1M", S["in"])
ax.set_row(7, 18)
ax.merge_range(7, 0, 7, 3, "  RECEIVE — LIVE MARKET", S["secI"])
ax.merge_range(7, 5, 7, 10, "  CONTRIBUTE — YOUR AXE", S["secC"])
for c, t in enumerate(["PAIR", "SPOT", "ATM", "RR25", "", "MY ATM", "AXE", "SIZE", "@Δ", "MARK", "1M ATM PX"]):
    ax.write(8, c, t, S["ull"] if c == 0 else S["ul"])
pairs = [("EURUSD", 10.6, "Buy"), ("GBPUSD", 9.4, ""), ("USDJPY", 11.2, "Sell"), ("AUDUSD", 12.1, ""),
         ("USDCHF", 8.9, ""), ("USDCAD", 8.2, "Buy"), ("NZDUSD", 12.6, ""), ("EURGBP", 7.8, ""),
         ("EURJPY", 10.9, "Sell"), ("EURCHF", 6.9, "")]
p0 = 9
for i, (pair, atm, axe) in enumerate(pairs):
    r = p0 + i; r1 = r + 1; ax.set_row(r, 17)
    ax.write(r, 0, pair, S["pair"])
    wf(ax, r, 1, f'=CELNET.SERIES("{pair}","SPOT")', "num")
    wf(ax, r, 2, f'=CELNET.SERIES("{pair}","ATM",$G$6)', "pct")
    wf(ax, r, 3, f'=CELNET.SERIES("{pair}","RR",$G$6,25)', "pct")
    ax.write(r, 5, atm, S["inp"]); ax.write(r, 6, axe, S["axe"])
    ax.write(r, 7, (25 if axe else ""), S["inc"]); ax.write(r, 8, "25Δ", S["in"])
    wf(ax, r, 9, f'=CELNET.MARK("{pair}",$G$6,"ATM",F{r1}/100,"VV","axe "&G{r1})', "val")
    wf(ax, r, 10, f'=CELNET.PRICE("{pair}",$G$6,"ATM","C",$B$6*1000000)', "cor")
ax.data_validation(p0, 6, p0 + len(pairs) - 1, 6, {"validate": "list", "source": ["Buy", "Sell", ""]})
stamp(ax, p0 + len(pairs) + 1, N)

# ============================================================= Structuring
M = 8
st = wb.add_worksheet("Structuring"); st.set_tab_color("#3A4DB0")
setup(st, [20, 14, 14, 14, 14, 14, 14, 14, 14])
brand(st, "Structuring", M)
st.write(5, 0, "PAIR", S["lab"]); st.write(5, 1, "EURUSD", S["inl"])
st.write(5, 3, "NOTIONAL", S["lab"]); st.write(5, 4, 1000000, S["inc"])
sec(st, 7, "Vanilla & strategy — PRICE / GREEKS", M)
st.write(8, 1, "PREMIUM", S["ul"]); st.write(8, 2, "GREEKS (spill →)", S["ull"])
rows = [("Vanilla call 1M ATM", '=CELNET.PRICE($B$6,"1M","ATM","C",$E$6)', '=CELNET.GREEKS($B$6,"1M","ATM","C",$E$6)'),
        ("Vanilla put 3M 25Δ", '=CELNET.PRICE($B$6,"3M","25dP","P",$E$6)', None),
        ("Risk reversal — call", '=CELNET.PRICE($B$6,"1M","25dC","C",$E$6)', None),
        ("Risk reversal — put", '=CELNET.PRICE($B$6,"1M","25dP","P",$E$6)', None)]
r = 9
for lbl, pr, gr in rows:
    st.set_row(r, 17); st.write(r, 0, lbl, S["pair"]); wf(st, r, 1, pr, "cor")
    if gr: wf(st, r, 2, gr, "val")
    r += 1
r += 1
sec(st, r, "Any product — INSTRUMENT → PRICE / GREEKS", M, coral=True); r += 1
exotics = [("Up-&-out call", "BARRIER", "3M", [("strike", "25dC"), ("callPut", "C"), ("barrier", 1.30), ("kind", "KNOCK_OUT"), ("side", "UP")]),
           ("One-touch", "TOUCH", "3M", [("kind", "OT"), ("barrier", 1.20), ("rebate", 100000)]),
           ("Asian call", "ASIAN", "3M", [("strike", 1.10), ("callPut", "C"), ("averaging", "DISCRETE")]),
           ("Variance swap", "VARSWAP", "3M", [("strikeVol", 0.11)])]
for lbl, product, tenor, terms in exotics:
    st.set_row(r, 17); st.write(r, 0, lbl, S["pair"])
    st.write(r, 1, "product", S["ul"]); st.write(r, 2, product, S["inl"])
    t0 = r + 1
    for j, (k, v) in enumerate(terms):
        st.write(t0 + j, 1, k, S["lab"]); st.write(t0 + j, 2, v, S["inl"] if isinstance(v, str) else S["in"])
    t1 = t0 + len(terms) - 1
    wf(st, r, 4, f'=CELNET.INSTRUMENT($B$6,C{r+1},B{t0+1}:C{t1+1},"{tenor}",$E$6)', "val")
    st.write(r, 5, "premium", S["ul"]); wf(st, r, 6, f"=CELNET.PRICE(E{r+1})", "cor")
    st.write(r, 7, "greeks", S["ul"]); wf(st, r, 8, f"=CELNET.GREEKS(E{r+1})", "val")
    r = t1 + 2
stamp(st, r + 1, M)

# ================================================================== Trading
T = 7
tr = wb.add_worksheet("Trading"); tr.set_tab_color("#1B7F4B")
setup(tr, [14, 12, 12, 10, 14, 14, 14, 14])
brand(tr, "Trading", T)
sec(tr, 5, "RFQ — bid · mid · offer (spills)", T)
for c, t in enumerate(["PAIR", "TENOR", "STRIKE/Δ", "C/P", "NOTIONAL"]):
    tr.write(6, c, t, S["ull"] if c == 0 else S["ul"])
for c, v, k in [(0, "EURUSD", "inl"), (1, "1M", "in"), (2, "ATM", "in"), (3, "C", "in"), (4, 1000000, "inc")]:
    tr.write(7, c, v, S[k])
tr.write(9, 0, "RFQ →", S["lab"]); wf(tr, 9, 1, "=CELNET.RFQ(A8,B8,C8,D8,E8)", "val")
sec(tr, 11, "Live stream (RTD)", T)
tr.write(12, 0, "STREAM →", S["lab"]); wf(tr, 12, 1, "=CELNET.SUBSCRIBE(A8,B8,C8,D8,E8)", "val")
sec(tr, 14, "Blotter", T)
for c, t in enumerate(["TIME", "PAIR", "STRUCTURE", "SIDE", "NOTIONAL", "PRICE", "LP"]):
    tr.write(15, c, t, S["ull"] if c == 0 else S["ul"])
stamp(tr, 18, T)

# ============================================================== Risk & Book
rk = wb.add_worksheet("Risk & Book"); rk.set_tab_color("#8A2BE2")
setup(rk, [18, 14, 14, 14, 14, 14, 14, 14])
brand(rk, "Risk & book", T)
rk.write(5, 0, "STATUS", S["lab"]); wf(rk, 5, 1, '=CELNET.STATUS()', "val")
rk.write(7, 0, "NUMERAIRE", S["lab"]); rk.write(7, 1, "USD", S["in"])
rk.write(8, 0, "RATES", S["lab"]); rk.write(8, 1, "EUR", S["in"]); rk.write(8, 2, 1.10, S["in"])
sec(rk, 10, "Positions — CELNET.POSITIONS (spills)", T)
wf(rk, 11, 0, '=CELNET.POSITIONS("FIRM")', "val")
sec(rk, 14, "Firm risk — CELNET.RISK", T)
wf(rk, 15, 0, '=CELNET.RISK("FIRM",B8,B9:C9)', "val")
sec(rk, 19, "By desk", T)
wf(rk, 20, 0, '=CELNET.RISK("DESK",B8,B9:C9)', "val")
sec(rk, 24, "Limits — CELNET.LIMITS", T)
wf(rk, 25, 0, '=CELNET.LIMITS("FIRM",B8,B9:C9)', "val")
stamp(rk, 28, T)

wb.close()
print("WROTE", OUT)
