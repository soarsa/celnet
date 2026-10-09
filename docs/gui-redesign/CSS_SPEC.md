

# CelNet Chronos 2026 — Production CSS System

## 1. Theme Variables

```css
/* ============================================================
   THEME VARIABLE DEFINITIONS
   Seamless tonal elevation system — zero white borders
   ============================================================ */

/* --- ACID CHARTREUSE --- */
[data-theme="acid-chartreuse"] {
  /* Canvas & Elevation Surfaces */
  --bg-canvas:       #0a0a0a;
  --bg-card:         #111110;
  --bg-panel:        #161614;
  --bg-inset:        #1c1c18;

  /* Borders — tonal, never white */
  --border-subtle:   rgba(163, 230, 53, 0.08);
  --border-accent:   rgba(163, 230, 53, 0.25);

  /* Shadow & Bevel */
  --shadow-card:     0 1px 3px rgba(0, 0, 0, 0.5),
                     0 4px 12px rgba(0, 0, 0, 0.35);
  --bevel-top:       inset 0 1px 0 rgba(163, 230, 53, 0.06);

  /* Semantic Palette */
  --accent-primary:  #a3e635;
  --accent-dim:      #4d7c0f;
  --accent-glow:     rgba(163, 230, 53, 0.12);
  --text-primary:    #f0fdf4;
  --text-secondary:  rgba(163, 230, 53, 0.55);
  --text-muted:      rgba(163, 230, 53, 0.30);

  /* Table Specifics */
  --table-row-alt:   rgba(163, 230, 53, 0.03);
  --table-header-bg: #1a1f12;
  --table-border:    rgba(163, 230, 53, 0.06);

  /* Interactive States */
  --hover-lift:      0 2px 8px rgba(0, 0, 0, 0.5),
                     0 8px 24px rgba(0, 0, 0, 0.4),
                     0 0 0 1px rgba(163, 230, 53, 0.12);
}

/* --- SAFETY ORANGE --- */
[data-theme="safety-orange"] {
  --bg-canvas:       #0a0908;
  --bg-card:         #12100e;
  --bg-panel:        #181513;
  --bg-inset:        #1e1a16;

  --border-subtle:   rgba(251, 146, 60, 0.08);
  --border-accent:   rgba(251, 146, 60, 0.25);

  --shadow-card:     0 1px 3px rgba(0, 0, 0, 0.5),
                     0 4px 12px rgba(0, 0, 0, 0.35);
  --bevel-top:       inset 0 1px 0 rgba(251, 146, 60, 0.06);

  --accent-primary:  #fb923c;
  --accent-dim:      #9a3412;
  --accent-glow:     rgba(251, 146, 60, 0.12);
  --text-primary:    #fff7ed;
  --text-secondary:  rgba(251, 146, 60, 0.55);
  --text-muted:      rgba(251, 146, 60, 0.30);

  --table-row-alt:   rgba(251, 146, 60, 0.03);
  --table-header-bg: #1f1610;
  --table-border:    rgba(251, 146, 60, 0.06);

  --hover-lift:      0 2px 8px rgba(0, 0, 0, 0.5),
                     0 8px 24px rgba(0, 0, 0, 0.4),
                     0 0 0 1px rgba(251, 146, 60, 0.12);
}

/* --- SOLAR AMBER --- */
[data-theme="solar-amber"] {
  --bg-canvas:       #0a0a08;
  --bg-card:         #11110e;
  --bg-panel:        #171714;
  --bg-inset:        #1d1d18;

  --border-subtle:   rgba(250, 204, 21, 0.07);
  --border-accent:   rgba(250, 204, 21, 0.22);

  --shadow-card:     0 1px 3px rgba(0, 0, 0, 0.5),
                     0 4px 12px rgba(0, 0, 0, 0.35);
  --bevel-top:       inset 0 1px 0 rgba(250, 204, 21, 0.05);

  --accent-primary:  #facc15;
  --accent-dim:      #854d0e;
  --accent-glow:     rgba(250, 204, 21, 0.10);
  --text-primary:    #fefce8;
  --text-secondary:  rgba(250, 204, 21, 0.50);
  --text-muted:      rgba(250, 204, 21, 0.28);

  --table-row-alt:   rgba(250, 204, 21, 0.025);
  --table-header-bg: #1a1a10;
  --table-border:    rgba(250, 204, 21, 0.05);

  --hover-lift:      0 2px 8px rgba(0, 0, 0, 0.5),
                     0 8px 24px rgba(0, 0, 0, 0.4),
                     0 0 0 1px rgba(250, 204, 21, 0.10);
}

/* --- MONOLITH --- */
[data-theme="monolith"] {
  --bg-canvas:       #09090b;
  --bg-card:         #0f0f12;
  --bg-panel:        #151518;
  --bg-inset:        #1b1b1f;

  --border-subtle:   rgba(161, 161, 170, 0.07);
  --border-accent:   rgba(161, 161, 170, 0.18);

  --shadow-card:     0 1px 3px rgba(0, 0, 0, 0.55),
                     0 4px 12px rgba(0, 0, 0, 0.4);
  --bevel-top:       inset 0 1px 0 rgba(255, 255, 255, 0.04);

  --accent-primary:  #a1a1aa;
  --accent-dim:      #52525b;
  --accent-glow:     rgba(161, 161, 170, 0.08);
  --text-primary:    #fafafa;
  --text-secondary:  rgba(161, 161, 170, 0.55);
  --text-muted:      rgba(161, 161, 170, 0.30);

  --table-row-alt:   rgba(255, 255, 255, 0.015);
  --table-header-bg: #131316;
  --table-border:    rgba(161, 161, 170, 0.05);

  --hover-lift:      0 2px 8px rgba(0, 0, 0, 0.55),
                     0 8px 24px rgba(0, 0, 0, 0.45),
                     0 0 0 1px rgba(161, 161, 170, 0.08);
}
```

---

## 2. Component Selectors

```css
/* ============================================================
   BASE CANVAS
   ============================================================ */
body,
.dashboard-root {
  background-color: var(--bg-canvas);
  color: var(--text-primary);
  -webkit-font-smoothing: antialiased;
}


/* ============================================================
   KPI CARD
   Elevation tier: card (layer 1)
   ============================================================ */
.kpi-card {
  background: var(--bg-card);
  border: 1px solid var(--border-subtle);
  border-radius: 8px;
  padding: 20px 24px;
  box-shadow: var(--shadow-card);

  /* Top bevel — simulates overhead light on a raised surface */
  box-shadow:
    var(--shadow-card),
    var(--bevel-top);

  /* Prevent any white bleed from compositing */
  isolation: isolate;

  transition:
    box-shadow 180ms ease,
    border-color 180ms ease,
    transform 180ms ease;
}

.kpi-card:hover {
  box-shadow: var(--hover-lift), var(--bevel-top);
  border-color: var(--border-accent);
  transform: translateY(-1px);
}

/* KPI inner typography */
.kpi-card .kpi-label {
  font-size: 0.7rem;
  font-weight: 600;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--text-muted);
  margin-bottom: 6px;
}

.kpi-card .kpi-value {
  font-size: 1.75rem;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  color: var(--accent-primary);
  line-height: 1.1;
}

.kpi-card .kpi-delta {
  font-size: 0.75rem;
  font-weight: 500;
  margin-top: 4px;
}
```