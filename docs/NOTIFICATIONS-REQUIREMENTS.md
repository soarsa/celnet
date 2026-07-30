# Configurable Trader Notifications — Requirements / Design

> Status: **requirements + design for review** (2026-07-28). No code yet. This
> **EXTENDS** the already-shipped notification stack (the `Notification` push
> stream + `NotificationCenter` + WebAudio cue + desktop "growl"), it does **not**
> reinvent it. Every existing seam is cited `file:line` (current as of this doc).
> All sound-asset choices obey guardrail **#7** (OSS/permissive only — for audio
> that means **CC0 / public-domain / self-generated**, never copyrighted
> game/movie/gunshot recordings) and naming obeys guardrail **#8** (purpose-named,
> vendor-neutral). Every external claim is cited in the **Appendix A** source table.

## 1. Concept & goal

Traders on a live desk want their alerting to be **theirs**: which events beep, what
each one *sounds* like, and where it shows up (a screen toast vs an OS banner when
they've tabbed away). Trading-floor culture leans on distinctive fill sounds — the
user explicitly asked for a "multi-kill"-style streak stinger, a percussive
"shot"-style hit for a big fill, and other playful cues. **We deliver that vibe with
a licensing-clean sound set** (CC0 packs + procedurally-generated Web Audio cues —
§4), never by ripping iconic game/movie audio.

Three deliverables, all layered on the existing notification surface:
1. **A richer event taxonomy** — orders, fills, executions, RFQ-manual-intervention,
   and the existing exception/quote-lifecycle events, each individually addressable
   (§3).
2. **A licensing-safe sound library** — a named CC0/generated sound set mapped to
   events, preloaded, mute/volume/reduced-motion aware (§4).
3. **Per-event, per-channel configuration** — a trader picks, *per event type*:
   on/off, **which sound**, **which channel** (in-app growl toast vs OS desktop
   banner), and volume/severity — persisted per user (§5, §6).

### Decisions proposed (for the 2026-07-28 review)
- **Primary sound engine = the existing procedural Web Audio synth** (`playCue`,
  `useNotificationStore.ts:202`), extended into a small **synth sound-kit** — zero
  license, zero bundle weight, already shipping. Bundled **CC0 sample files**
  (Kenney UI Audio) are the optional "richer" tier for traders who want recorded
  character. **No attribution-required or non-CC0 sources ship** (§4.3 verdict).
- **Channel rule = focus-aware** (already implemented, keep it): **OS desktop banner
  only when the tab is hidden/unfocused; in-app growl toast when it's on screen** —
  never both (`useDesktopNotifications.ts:97,186`). Per-event config layers *on top*
  of this rule (a trader can still say "fills: desktop only / never").
- **Config storage = client-side `localStorage`** (extend `AppSettings`,
  `settingsSchema.ts`), matching the shipped pattern; a server-side `UserDef`-backed
  sync is an explicit **open item** (§8) for cross-device persistence.

## 2. Architecture fit — the real seams (build ON these, don't reinvent)

The notification pipeline already exists end-to-end. This capability adds **breadth**
(more event kinds), **variety** (a sound kit), and **granularity** (per-event config).

- **Wire contract** — `crates/celnet-proto/proto/celnet.proto`:
  - `enum NotificationKind` (line **6818**): `RFQ_RECEIVED`(1), `IOI_RECEIVED`(2),
    `REQUEST_WITHDRAWN`(3), `REQUEST_EXPIRED`(4), `QUOTE_ACCEPTED`(5) *(= a quote was
    lifted → a deal booked)*, `QUOTE_REJECTED`(6), `MANUAL_INTERVENTION_REQUIRED`(7).
  - `enum ManualInterventionReason` (line **6842**): `UNCONFIGURED_TENOR`(1),
    `CREDIT_RISK_BREAK`(2), `UNKNOWN_SECURITY`(3), `PRICING_FAILURE`(4).
  - `message Notification` (line **6856**): `notification_id, kind, at_nanos,
    request_id, desk, counterparty, request_kind, headline, detail, alert_worthy,
    reason`. **`alert_worthy` (line 6875) is the authoritative server-owned POPUP
    gate** — the whole "exception-only" behaviour hangs off this bool.
  - `NotificationScope` (6884) + `StreamNotificationsRequest` (6888) — a per-desk
    entitlement-filtered subscription.
- **Server publish / fan-out** — `crates/celnet-server/src/services/desk/`:
  - `RfqDeskEdge::publish_notification` (`mod.rs:383`) — the fan-out to matching
    subscribers; `submit_publishes_notification` (`mod.rs:1592`);
    `ingest_fix_rfq_manual_intervention_is_alert_worthy_with_reason` (`mod.rs:1710`)
    — where a non-auto-priceable RFQ becomes an `alert_worthy=true` kind-7 event.
  - `services/desk/notify.rs` — the notification builder/publisher module.
- **Encode/transport** — `ws/codec.rs::notification_to_json` (**2758**) +
  `hand_notification` (**5275**); `ws/generated_codec.rs::encode_notification`
  (**4972**) (descriptor-driven — a new enum arm auto-adapts).
- **Client SDK** — `crates/celnet-client/src/notify.rs::open_stream` (**188**);
  `Client::stream_notifications` (`lib.rs:1057`).
- **GUI decode** — `gui/src/data/wsCodec.ts::notificationFromWire` (**1844**);
  types in `gui/src/data/contract.ts` (`Notification`, `NotificationKind`).
- **GUI store (the heart to extend)** — `gui/src/hooks/useNotificationStore.ts`:
  - `planNotification` (**102**) — the pure gate: `alert_worthy` drives
    toast/sound/growl; `shouldSuppress` (**70**) applies the `minQty` threshold;
    `MAX_HISTORY=50`, `MAX_TOASTS=4`, `TOAST_TTL_MS=6000` (**30–34**).
  - `playCue` (**202**) — the **existing procedural WebAudio cue** (a single sine,
    740 Hz for pending / 523 Hz for terminal, ~0.2 s, gain-capped). This is the
    beachhead for the sound kit (§4.2).
  - Auto-clear: `pruneOnTerminal` (**141**) + `expireByTtl` (**158**).
- **GUI desktop escalation** — `gui/src/hooks/useDesktopNotifications.ts`: the
  Web Notifications wrapper — `tabIsAway()` (**97**, `visibilityState==="hidden" ||
  !document.hasFocus()`), the focus-aware `notify` (**184**), the once-only
  gesture/first-mount permission request (**137,157**), `localStorage` mute pref
  (independent of the OS grant).
- **GUI surfaces** — `gui/src/components/NotificationCenter.tsx` (bell + dropdown +
  toast render), `SettingsPanel.tsx` (Alerts / Sound / Thresholds / Notifications
  sections — the config home), `gui/src/lib/notificationText.ts`
  (`manualInterventionText`, `notionalMagnitude`, `compactNotionals`).
- **Client-side prefs** — `gui/src/settings/settingsSchema.ts`: `AppSettings`
  `{ alertsEnabled, soundsEnabled, volume, minQty, growlEnabled, autoClearCompleted,
  autoClearTtlSeconds }`, `localStorage` key `celnet.settings.v1`, **forward-safe
  merge-over-defaults** loader (so adding fields is safe).

**Takeaway:** the transport, fan-out, focus-aware channel routing, WebAudio cue,
permission flow, and a settings panel already exist. This work is **(a)** a handful
of additive `NotificationKind` arms, **(b)** a sound kit replacing the single beep,
and **(c)** a per-event config model replacing today's 3 global toggles.

## 3. Event taxonomy — map to the existing contract

The requirement names five event families. Four map to existing/near-existing
`NotificationKind` arms; **orders** and **own-fills** need additive arms (guardrail 9
allows additive enum growth — "new reasons append with the next number", proto
comment line 6841; there is one current contract, no versioning).

| Trader event family | Meaning | Existing `NotificationKind` | Action |
|---|---|---|---|
| **Incoming order** | A firm order routed to the desk | — (RFQ/IOI only today) | **NEW** `ORDER_RECEIVED` |
| **Fill / execution** | Own order (partially) filled | partially: `QUOTE_ACCEPTED`(5) covers *desk-quote lifted → deal booked* | **NEW** `FILL` (+keep 5 for quote-lift) |
| **RFQ / quote request** | An RFQ/IOI landed to work | `RFQ_RECEIVED`(1), `IOI_RECEIVED`(2) | reuse |
| **Manual intervention** | Can't auto-price → needs a human | `MANUAL_INTERVENTION_REQUIRED`(7) + `ManualInterventionReason` | reuse |
| **Exception / lifecycle** | Withdrawn / expired / rejected / credit break | `REQUEST_WITHDRAWN`(3), `REQUEST_EXPIRED`(4), `QUOTE_REJECTED`(6); reasons `CREDIT_RISK_BREAK`/`PRICING_FAILURE` | reuse |

- **Severity** is derived, not a new wire field: `MANUAL_INTERVENTION_REQUIRED` +
  any `ManualInterventionReason` and `QUOTE_REJECTED`/exceptions are **urgent**;
  `*_RECEIVED`/`FILL`/`QUOTE_ACCEPTED` are **informational**. Severity picks the
  **default** sound/channel per event (a trader can override).
- **`alert_worthy` stays the server's popup gate.** Per-event *client* config narrows
  it further (a trader can silence an alert-worthy kind), but can never force a popup
  the server marked quiet — the server remains the source of truth for "this matters".
- The **booking seams** that would emit the new `FILL`/`ORDER_RECEIVED` events are the
  same ones the FI-risk-routing doc identifies: ESP fills at
  `services/stream.rs::record_booked_position`, RFQ fills at
  `services/quote.rs::accept_quote` / `services/desk/mod.rs::accept_desk_quote` — so
  the two capabilities share an interception point.

## 4. Sound library (licensing-first)

### 4.1 Guardrail #7 verdict per source (the licensing table)

Audio is code-adjacent IP; guardrail #7 forbids proprietary/commercial and (for
sound) copyrighted recordings. The **only unconditionally safe** categories are
**CC0 / public-domain** samples and **self-generated** audio. Attribution-required
licenses are treated as **AVOID** for a shipped trading product (a terminal has
nowhere natural to carry third-party audio credits, and the spirit of #7 is a clean
OSS/permissive bill of materials).

| Source | License | Attribution? | Redistribute bundled? | **Verdict** |
|---|---|---|---|---|
| **Kenney.nl** (Interface Sounds 100 / UI Audio 50) | **CC0 1.0** | No | Yes | **✅ USE — primary sample pack** |
| **Freesound.org** *filtered to CC0* | **CC0** (per-sound) | No | Yes | **✅ USE — must verify each sound is CC0, not CC-BY/CC-BY-NC** |
| **Self-generated Web Audio** (oscillator + gain envelope) | none (our IP) | n/a | Yes | **✅ USE — recommended default engine** |
| Freesound.org CC-BY / CC-BY-NC sounds | CC-BY / CC-BY-NC | Yes / NC | — | **❌ AVOID** (attribution; NC bars commercial use) |
| **Mixkit** free SFX | Mixkit license (royalty-free, no attribution) | No | **No — may not redistribute "as-is" without significant change** | **⚠️ AVOID** (bundling app assets = redistribution as-is; not CC0 → outside the strict guardrail) |
| **Pixabay** Content License | royalty-free, no attribution | No | **No — cannot redistribute as-is** | **⚠️ AVOID** (same redistribution clause; not CC0) |
| **ZapSplat** Standard (free) | attribution required unless paid "Gold" | **Yes** | limited | **❌ AVOID** (attribution; paid tier = commercial ⇒ #7) |

**Net:** ship only **Kenney CC0** samples + **CC0-verified Freesound** picks +
**procedurally-generated** cues. Mixkit/Pixabay are permissive-*enough* for many
projects but their "no redistribution as-is" clause is a poor fit for bundling raw
files into a distributed app and they are not public-domain — so they're excluded to
keep the audio BOM strictly CC0/self-made. (Cited: Appendix A rows 1–7.)

### 4.2 The recommended named sound set (event → sound)

Nine core cues + one opt-in fun pack, each realizable **either** as a Kenney CC0
sample **or** a synthesized cue (so a zero-file build still works). The "multi-kill"
and "gunshot" *vibes* are delivered licensing-clean:

| # | Sound name (purpose-named) | Event | Character | CC0/generated realization |
|---|---|---|---|---|
| 1 | `fill-confirm` | single `FILL` | short rising two-tone "confirm" blip | Kenney UI *confirmation_001* (CC0) **or** synth 660→880 Hz |
| 2 | `fill-streak` **("multi-kill" vibe)** | consecutive fills within N s | an **escalating arpeggio** that climbs a step per streak count (double→triple→multi) | **synth** — a 3–5 note ascending run, pitch ladder keyed to the streak counter (zero-license; this *is* the safe "multi-kill" answer) |
| 3 | `fill-block` **("shot" vibe)** | large/block `FILL` (> size threshold) | a **punchy percussive transient** (thud/knock) | **synth** — short filtered white-noise burst through a lowpass + fast decay (a "shot" with no gun recording) **or** Kenney *impact* (CC0) |
| 4 | `order-in` | `ORDER_RECEIVED` | soft marimba/pluck | Kenney UI *click/pluck* (CC0) or synth triangle pluck |
| 5 | `rfq-work` | `RFQ_RECEIVED`/`IOI_RECEIVED` | gentle neutral "incoming" tick | Kenney UI *tick* (CC0) or synth 523 Hz blip |
| 6 | `needs-you` **(urgent)** | `MANUAL_INTERVENTION_REQUIRED` | distinct attention two-note, insistent not alarming | synth 880↔988 Hz double **or** Kenney *maximize* (CC0) |
| 7 | `won` | `QUOTE_ACCEPTED` (deal booked) | pleasant rising success chime | Kenney *confirmation up* (CC0) or synth major triad up |
| 8 | `lost` | `QUOTE_REJECTED` | gentle descending "declined" | synth 523→349 Hz down |
| 9 | `lapsed` | `REQUEST_WITHDRAWN`/`REQUEST_EXPIRED` | soft neutral low tick | Kenney *click* (CC0) or synth 392 Hz short |
| 10 | `celebrate` *(opt-in, off by default)* | streak milestone / playful | cheeky "boing/pop" — the "funny" pack | CC0-only cosmetic pack (Kenney/Freesound CC0); **opt-in** so it never annoys |

Design notes: the **streak/multi-fill** ladder and the **block "shot"** are the two
the trader most wanted, and both are **synthesized** — so they're not only
license-free but *parametric* (the ladder height literally encodes the streak count),
which recorded audio can't do.

### 4.3 Format, delivery, accessibility

- **Format:** short (< 500 ms) **mono**, **OGG Vorbis** (open codec) with a small
  **WAV/MP3** fallback; each file a few KB. Prefer OGG (fully open) to sidestep any
  MP3 patent-era concern; WAV is uncompressed-PCM and license-free.
- **Preload:** decode each sample **once** into an `AudioBuffer` at sign-in (behind
  the first user gesture that unlocks the `AudioContext`) and replay the buffer —
  never re-fetch/re-decode per event. The synth cues need no assets at all.
- **Reuse the existing lazy shared `AudioContext`** (`useNotificationStore.ts:171`)
  and its `suspended`→`resume()` guard; keep every audio call in a `try/catch`
  no-op (as `playCue` already is) so audio never throws into React.
- **Accessibility & etiquette:**
  - **Global mute** (existing `soundsEnabled`) + **per-user volume**
    (existing `volume` 0–100) + **per-event on/off** (new).
  - **Never audio-only** — every sound is paired with a visible toast/inbox item
    (audio is an *enhancement*, not the sole signal — WCAG).
  - Respect **`prefers-reduced-motion`** for toast entrance/exit animation (fall back
    to instant show/hide); a "reduce motion" trader still gets the toast, just no
    slide/bounce. Sounds are gated by the explicit mute, not by reduced-motion.
  - **Rate-limit / de-dupe** rapid bursts (a wall of fills shouldn't be a machine-gun
    of beeps) — coalesce within a short window into the `fill-streak` cue.

## 5. Delivery channels — desktop vs in-app growl

Two channels already coexist; this section reviews both and fixes the rule.

### 5.1 In-app "growl" toast (on-screen)
Transient stacked cards from the `NotificationCenter` (`toasts`, `MAX_TOASTS=4`,
`TOAST_TTL_MS=6000`). Best practice confirmed by the toast-library survey (Appendix A
rows 12): **stacking**, **auto-dismiss**, **severity styling**, **focus-aware** (only
when the tab is visible), and correct ARIA roles — `role="status"` (polite) for
informational, `role="alert"` (assertive) for urgent. **Recommendation: keep Celnet's
native toast** (it already stacks, TTLs, and integrates the store) rather than adopt
Sonner/react-hot-toast/Radix — but *port their patterns*: add `role="alert"` for
urgent kinds, `aria-live` regions, hover-to-pause-dismiss, and a manual close ×.
(Sonner is the current community default and the reference for polish; react-hot-toast
is the 5 KB minimal; Radix Toast is the a11y primitive — we match their behaviour
natively to avoid a dependency and keep bundle discipline, guardrail-web perf.)

### 5.2 OS desktop notifications (Web Notifications API)
- **API:** `new Notification(title, opts)` / `ServiceWorkerRegistration.showNotification`;
  permission via **`Notification.requestPermission()`** which **must be triggered by a
  user gesture** and returns `granted|denied|default`. Already wrapped in
  `useDesktopNotifications.ts` (gesture path on the toggle, one guarded first-mount
  auto-ask, legacy-callback tolerated).
- **Strengths:** shown **outside the viewport, even when the tab is unfocused or the
  browser is minimized** — the only way to reach a tabbed-away trader.
- **Limitations to document (Appendix A rows 8–10):** permission must be
  user-granted; **no cross-origin iframe** requests (Chrome/Firefox); **OS-dependent
  rendering** and **no reliable per-notification sound control** on some OSes (so we
  play our own cue in-page, not via the OS notification's sound); auto-close timing is
  OS-controlled; **Push API** (server-push to a closed app via a push service) is
  **out of scope** — CelNet's GUI is an open, connected app, so the in-page WS stream
  already delivers events; we only need the *display* surface, not web-push transport.
- **When it helps vs annoys:** helps when the trader has tabbed to another
  app/window; annoys (double-notifies) when the tab is on screen.

### 5.3 The focus-aware rule (keep + generalize)
**Desktop banner iff the tab is away; in-app toast iff it's on screen — never both.**
Implemented today via `tabIsAway()` (`useDesktopNotifications.ts:97`,
`visibilityState==="hidden" || !document.hasFocus()`) gating `notify` (**186**), while
the toast fires unconditionally in-store. Per-event config layers on top: a trader may
set an event to *desktop-only*, *toast-only*, *both-when-away/toast-when-here*
(the default), or *off*. A **`visibilitychange`** listener also clears the unread
favicon badge / dismisses stale desktop banners when the trader returns (Appendix A
row 11).

### 5.4 Recommendation
**Keep the existing focus-aware, dual-channel design and make it per-event
configurable.** Desktop = the away-escalation; in-app growl = the on-screen default.
Do **not** add a toast library or the Push API. Extend, don't replace.

## 6. Configuration model & UI

### 6.1 Data model (extend `AppSettings`, client-side)
Replace the three global toggles with a **per-event map** plus the existing globals as
masters. New `localStorage` key `celnet.settings.v2` (the loader already
merges-over-defaults, so a v1 blob upgrades cleanly — `settingsSchema.ts:71`).

```
NotificationEventType =                    // the configurable rows (§3)
  | OrderReceived | Fill | FillBlock | RfqReceived | IoiReceived
  | ManualIntervention | QuoteAccepted | QuoteRejected | RequestLapsed

PerEventPref {
  enabled: boolean,                        // master on/off for this event
  sound: SoundId | "none",                 // pick from §4.2 (or silent)
  channels: { toast: boolean, desktop: boolean },  // layered on the focus-aware rule
  volumeScale?: number,                    // optional 0–100 per-event trim of the global volume
}

AppSettings (v2) {
  // existing masters (kept):
  alertsEnabled, soundsEnabled, volume, minQty, growlEnabled,
  autoClearCompleted, autoClearTtlSeconds,
  // NEW:
  perEvent: Record<NotificationEventType, PerEventPref>,
  streakWindowMs: number,                  // coalesce fills into fill-streak (§4.2)
  reducedMotion?: "auto" | "on" | "off",   // toast animation (auto ⇒ prefers-reduced-motion)
}
```
Masters still win: `alertsEnabled=false` silences everything; `soundsEnabled=false`
mutes all cues; `growlEnabled`/OS-grant still gate the desktop channel; `minQty` still
suppresses sub-threshold events before per-event config is consulted
(`shouldSuppress`). `planNotification` (`useNotificationStore.ts:102`) grows to
consult `perEvent[kind]` instead of the single `alert_worthy && soundsEnabled` line.

### 6.2 UI — a "Notifications" settings table
Extend `SettingsPanel.tsx` (which already has Alerts / Sound / Thresholds /
Notifications sections) with a **per-event table**:

| Event | On | Sound | ▶ | Channel | Vol |
|---|---|---|---|---|---|
| Fill | ☑ | `fill-confirm ▾` | ▶ preview | Toast + Desktop-when-away ▾ | ▭ |
| Big fill | ☑ | `fill-block ▾` | ▶ | Toast ▾ | ▭ |
| Multi-fill streak | ☑ | `fill-streak ▾` | ▶ | Toast ▾ | ▭ |
| RFQ received | ☑ | `rfq-work ▾` | ▶ | Toast ▾ | ▭ |
| Needs manual pricing | ☑ | `needs-you ▾` | ▶ | Toast + Desktop ▾ | ▭ |
| Quote accepted (won) | ☑ | `won ▾` | ▶ | Toast ▾ | ▭ |
| Quote rejected (lost) | ☑ | `lost ▾` | ▶ | Toast ▾ | ▭ |
| … | | | | | |

- Each **sound cell is a dropdown** of the §4.2 kit (never free text) + a **▶ preview**
  button that plays the cue (drives the same code path as a live event).
- **Channel** is a dropdown: *Toast / Desktop / Both (focus-aware) / Off*.
- A **"Test notification"** button raises a sample toast + desktop banner + sound so a
  trader can confirm OS permission and volume before it matters live.
- The existing desktop-permission hint/toggle (`SettingsPanel.tsx:50,121`) stays; when
  any event's channel includes Desktop but permission is `default/denied`, show the
  inline "enable OS notifications" prompt.

### 6.3 Storage & scope
Client-side `localStorage` per browser/machine (matches today). **Open item (§8):** a
server-side `UserDef`-backed sync for cross-device parity — a trader who logs in on a
second terminal keeps their kit. The proto/`AuthService` CRUD precedent
(pricing-groups / desk config) is the template if we promote prefs server-side.

## 7. Phased plan & workstream breakdown (parallel-safe, disjoint files)

1. **Sound kit (GUI, self-contained)** — `gui/src/lib/soundKit.ts`: the synth cue
   generators (streak ladder, noise-burst "shot", chimes) + optional CC0
   `AudioBuffer` loader/preload; `SoundId` registry; unit tests (envelope shape,
   volume clamp, no-throw guards). *No server dep.*
2. **Per-event config model** — `settingsSchema.ts` v2 (`perEvent`, `streakWindowMs`,
   `reducedMotion`) + merge/clamp + tests (v1→v2 upgrade round-trips).
3. **Store wiring** — `useNotificationStore.ts`: `planNotification` consults
   `perEvent`; streak coalescing; route to `soundKit` instead of `playCue`; keep the
   focus-aware growl. Pure-decision tests extended.
4. **Settings UI** — `SettingsPanel.tsx` per-event table + preview/test buttons +
   reduced-motion + a11y roles on toasts (`NotificationCenter.tsx`).
5. **New event kinds (server + contract)** — additive `ORDER_RECEIVED` + `FILL`
   `NotificationKind` arms in `celnet.proto`; emit them at the booking seams
   (`stream.rs` ESP fill, `quote.rs`/`desk/mod.rs` RFQ fill); codec entries
   (`ws/codec.rs` + descriptor-driven `generated_codec.rs`); client `notify.rs` +
   `wsCodec.ts` decode; `contract.ts` types.
6. **Cross-client + e2e** — CLI/Excel/SDK notification parity for the new kinds; a
   live GUI check that each event lights the right toast/desktop/sound.

Phases 1–4 are **GUI-only (0 Rust delta)** and land without touching `Cargo.lock`;
phase 5 is the only server change. Each phase gated (`just t1` per crate; GUI
`npm run build`; `just t2` at land). Guardrail #7 enforced by shipping only CC0 files
(record their provenance in an `ASSETS.md`/ADR) + generated cues.

## 8. Open items for the next review
- **Server-side pref sync (`UserDef`)** — cross-device kit persistence vs today's
  per-browser `localStorage` (§6.3). Decision: client-only for v1, promote later?
- **`ORDER_RECEIVED`/`FILL` scope** — do we emit own-fill notifications for *every*
  fill (chatty) or only counterparty/desk-facing ones? Ties to the fill booking seam
  shared with FI-risk-routing.
- **Streak semantics** — window length, per-instrument vs per-desk streak counting,
  and the ladder ceiling (how high the `fill-streak` climbs).
- **CC0 sample vetting workflow** — a checklist + committed provenance record
  (`ASSETS.md`/ADR) proving each shipped `.ogg` is CC0 (guardrail #7 audit trail).
- **"Fun pack" governance** — the opt-in `celebrate` cosmetic pack: which CC0 sounds,
  and is a desk-admin allowed to disable it firm-wide.
- **Do-not-disturb / focus mode** — a global "mute all but urgent" toggle (a trader in
  a call) beyond per-event config.

## Appendix A — cited sources (verified 2026-07-28)

| # | Claim | Source |
|---|---|---|
| 1 | Freesound hosts CC0/CC-BY/CC-BY-NC; filter to CC0 ⇒ public domain, no attribution | https://freesound.org/ ; https://en.wikipedia.org/wiki/Freesound |
| 2 | CC0 = public-domain dedication, no attribution required, commercial OK | https://freesound.org/forum/legal-help-and-attribution-questions/34470/ |
| 3 | Kenney "Interface Sounds" (100) & "UI Audio" (50) are **CC0 1.0**, credit optional | https://kenney.nl/assets/interface-sounds ; https://kenney.nl/assets/ui-audio ; https://github.com/Calinou/kenney-ui-audio/blob/master/LICENSE.txt |
| 4 | Mixkit free SFX: no attribution, commercial OK, **but may not redistribute as-is / register / claim** | https://mixkit.co/license/ ; https://mixkit.co/free-sound-effects/ |
| 5 | Pixabay Content License: no attribution, commercial OK, **cannot resell/redistribute as-is** | https://pixabay.com/service/license-summary/ ; https://pixabay.com/service/faq/ |
| 6 | ZapSplat Standard (free) **requires attribution** ("Sound effects obtained from zapsplat.com") unless paid Gold | https://www.zapsplat.com/license-type/standard-license/ ; https://www.zapsplat.com/how-to-credit-us/ |
| 7 | ZapSplat also hosts a CC0 1.0 subset (no attribution) | https://www.zapsplat.com/license-type/cc0-1-0-universal/ |
| 8 | `Notification.requestPermission()` must be a user gesture; returns granted/denied/default | https://developer.mozilla.org/en-US/docs/Web/API/Notification/requestPermission_static |
| 9 | Notifications show outside the viewport even when the tab is unfocused/minimized; usage & `ServiceWorkerRegistration.showNotification` | https://developer.mozilla.org/en-US/docs/Web/API/Notifications_API/Using_the_Notifications_API ; https://notifications.spec.whatwg.org/ |
| 10 | Chrome/Firefox block notification permission requests from cross-origin iframes | https://developer.mozilla.org/en-US/docs/Web/API/Notification/requestPermission_static |
| 11 | Focus-aware pattern: `document.visibilityState` + `visibilitychange` → desktop when hidden, toast when visible; clear favicon/badge on return | https://developer.mozilla.org/en-US/docs/Web/API/Document/visibilityState ; https://web.dev/pagevisibility-intro/ ; https://reactuse.com/blog/react-browser-tab-ux/ |
| 12 | Toast libraries: Sonner (shadcn default, accessible, stacked), react-hot-toast (~5 KB, role=status/alert), Radix Toast (a11y primitive) | https://blog.logrocket.com/react-toast-libraries-compared-2025/ |
| 13 | Web Audio API: oscillator + gain envelope synthesizes UI sounds with zero dependencies / tiny payload | https://developer.mozilla.org/en-US/docs/Web/API/Web_Audio_API ; https://dev.to/hexshift/how-to-build-a-zero-dependency-audio-synth-in-the-browser-using-web-audio-api-1bp5 |
