

## Module 1: QUANT_AUDIO

```javascript
const QUANT_AUDIO = (() => {
  let ctx = null;
  let muted = localStorage.getItem('chronos_muted') === 'true';

  const ensureCtx = () => {
    if (!ctx) {
      ctx = new AudioContext();
    }
    if (ctx.state === 'suspended') {
      ctx.resume();
    }
    return ctx;
  };

  const initOnGesture = () => {
    const handler = () => {
      ensureCtx();
      document.removeEventListener('click', handler);
      document.removeEventListener('keydown', handler);
      document.removeEventListener('touchstart', handler);
    };
    document.addEventListener('click', handler, { once: false });
    document.addEventListener('keydown', handler, { once: false });
    document.addEventListener('touchstart', handler, { once: false });
  };

  initOnGesture();

  const isMuted = () => muted;

  const createGain = (audioCtx, gainValue, connectTo) => {
    const g = audioCtx.createGain();
    g.gain.value = gainValue;
    g.connect(connectTo);
    return g;
  };

  const tick = (direction = 'up') => {
    if (muted) return;
    const ac = ensureCtx();
    const now = ac.currentTime;

    const osc = ac.createOscillator();
    osc.type = direction === 'up' ? 'sine' : 'triangle';
    osc.frequency.value = 800 + Math.random() * 400; // 800-1200Hz

    const gain = createGain(ac, 0.04, ac.destination);
    gain.gain.setValueAtTime(0.04, now);
    gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.01); // <10ms decay

    osc.connect(gain);
    osc.start(now);
    osc.stop(now + 0.015);
  };

  const sweep = () => {
    if (muted) return;
    const ac = ensureCtx();
    const now = ac.currentTime;

    // White noise burst through bandpass
    const bufferSize = ac.sampleRate * 0.05;
    const buffer = ac.createBuffer(1, bufferSize, ac.sampleRate);
    const data = buffer.getChannelData(0);
    for (let i = 0; i < bufferSize; i++) {
      data[i] = (Math.random() * 2 - 1);
    }

    const source = ac.createBufferSource();
    source.buffer = buffer;

    const bandpass = ac.createBiquadFilter();
    bandpass.type = 'bandpass';
    bandpass.frequency.value = 420;
    bandpass.Q.value = 6;

    const gain = createGain(ac, 0.06, ac.destination);
    gain.gain.setValueAtTime(0.06, now);
    gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.05);

    source.connect(bandpass);
    bandpass.connect(gain);
    source.start(now);
    source.stop(now + 0.06);
  };

  const rfqChime = () => {
    if (muted) return;
    const ac = ensureCtx();
    const now = ac.currentTime;

    const freqs = [587, 880];
    freqs.forEach((freq) => {
      const osc = ac.createOscillator();
      osc.type = 'sine';
      osc.frequency.value = freq;

      const gain = createGain(ac, 0.10, ac.destination);
      gain.gain.setValueAtTime(0.10, now);
      gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.15); // 150ms exponential decay

      osc.connect(gain);
      osc.start(now);
      osc.stop(now + 0.18);
    });
  };

  const volShockAlert = () => {
    if (muted) return;
    const ac = ensureCtx();
    const now = ac.currentTime;

    const osc = ac.createOscillator();
    osc.type = 'sine';
    osc.frequency.setValueAtTime(180, now);
    osc.frequency.exponentialRampToValueAtTime(90, now + 0.3); // descending drone

    const gain = createGain(ac, 0.08, ac.destination);
    gain.gain.setValueAtTime(0.08, now);
    gain.gain.linearRampToValueAtTime(0.0001, now + 0.3);

    osc.connect(gain);
    osc.start(now);
    osc.stop(now + 0.35);
  };

  const toggleMute = () => {
    muted = !muted;
    localStorage.setItem('chronos_muted', String(muted));

    const btn = document.querySelector('[data-mute-btn]') ||
                document.querySelector('.mute-btn') ||
                document.getElementById('mute-btn');
    if (btn) {
      btn.textContent = muted ? '🔇 MUTED' : '🔊 LIVE';
      btn.classList.toggle('is-muted', muted);
      btn.setAttribute('aria-pressed', String(muted));
    }

    return muted;
  };

  // Sync button state on load
  const syncBtn = () => {
    const btn = document.querySelector('[data-mute-btn]') ||
                document.querySelector('.mute-btn') ||
                document.getElementById('mute-btn');
    if (btn) {
      btn.textContent = muted ? '🔇 MUTED' : '🔊 LIVE';
      btn.classList.toggle('is-muted', muted);
    }
  };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', syncBtn);
  } else {
    syncBtn();
  }

  return Object.freeze({
    tick,
    sweep,
    rfqChime,
    volShockAlert,
    toggleMute,
    get muted() { return muted; },
  });
})();
```

## Module 2: MARKET_PULSE_AND_ENTROPY

```javascript
const MARKET_PULSE_AND_ENTROPY = (() => {
  // --- State ---
  let regime = 'CALM'; // 'CALM' | 'VOL_SHOCK'
  let spot = 5284.50;
  let tickerInterval = null;
  const BASE_TICK_MS = 400;
  const SHOCK_TICK_MS = 90;

  const COUNTERPARTIES = [
    'CITADEL', 'JUMP', 'VIRTU', 'OPTIVER', 'FLOW-DM', 'SIG', 'IMC',
    'TOWER', 'GTS', 'HUDSON', 'JANE-ST', 'DRW', 'AKUNA', 'WOLVERINE'
  ];

  // --- Brownian drift ---
  const drift = () => {
    const magnitude = regime === 'CALM'
      ? 0.25 + Math.random() * 1.25   // +/- 0.25 to 1.50
      : 0.50 + Math.random() * 3.00;  // wider in vol shock
    const direction = Math.random() < 0.5 ? -1 : 1;
    spot += direction * magnitude;
    // Soft mean-reversion toward 5284.50
    spot += (5284.50 - spot) * 0.002;
    spot = Math.round(spot * 100) / 100;
    return spot;
  };

  // --- Flash helper ---
  const flashCell = (element, isUp) => {
    if (!element) return;
    const cls = isUp ? 'cell-flash-up' : 'cell-flash-down';
    element.classList.add(cls);
    setTimeout(() => element.classList.remove(cls), 200);
  };

  // --- L2 Order Book ladder ---
  const generateLadder = (mid) => {
    const levels = 8;
    const asks = [];
    const bids = [];
    for (let i = 1; i <= levels; i++) {
      const spread = 0.25 * i + Math.random() * 0.15;
      const askSize = Math.floor(5 + Math.random() * 120);
      const bidSize = Math.floor(5 + Math.random() * 120);
      asks.push({
        price: Math.round((mid + spread) * 100) / 100,
        size: askSize,
      });
      bids.push({
        price: Math.round((mid - spread) * 100) / 100,
        size: bidSize,
      });
    }
    // Asks sorted ascending, bids sorted descending
    asks.sort((a, b) => a.price - b.price);
    bids.sort((a, b) => b.price - a.price);
    return { asks, bids };
  };

  const updateOrderBook = (mid) => {
    const { asks, bids } = generateLadder(mid);

    const askContainer = document.querySelector('.ob-asks') || document.getElementById('ob-asks');
    const bidContainer = document.querySelector('.ob-bids') || document.getElementById('ob-bids');

    if (askContainer) {
      const rows = askContainer.querySelectorAll('.ob-row');
      asks.forEach((level, i) => {
        if (rows[i]) {
          const priceCell = rows[i].querySelector('.ob-price');
          const sizeCell = rows[i].querySelector('.ob-size');
          if (priceCell) {
            const oldPrice = parseFloat(priceCell.textContent);
            priceCell.textContent = level.price.toFixed(2);
            if (!isNaN(oldPrice) && level.price !== oldPrice) {
              flashCell(priceCell, level.price > oldPrice);
            }
          }
          if (sizeCell) {
            const oldSize = parseInt(sizeCell.textContent, 10);
            sizeCell.textContent = level.size;
            if (!isNaN(oldSize) && level.size !== oldSize) {
              flashCell(sizeCell, level.size > oldSize);
            }
          }
        }
      });
    }

    if (bidContainer) {
      const rows = bidContainer.querySelectorAll('.ob-row');
      bids.forEach((level, i) => {
        if (rows[i]) {
          const priceCell = rows[i].querySelector('.ob-price');
          const sizeCell = rows[i].querySelector('.ob-size');
          if (priceCell) {
            const oldPrice = parseFloat(priceCell.textContent);
            priceCell.textContent = level.price.toFixed(2);
            if (!isNaN(oldPrice) && level.price !== oldPrice) {
              flashCell(priceCell, level.price > oldPrice);
            }
          }
          if (sizeCell) {
            const oldSize = parseInt(sizeCell.textContent, 10);
            sizeCell.textContent = level.size;
            if (!isNaN(oldSize) && level.size !== oldSize) {
              flashCell(sizeCell, level.size > oldSize);
            }
          }
        }
      });
    }
  };

  // --- Time & Sales tape ---
  const appendExecution = (price) => {
    const tape = document.querySelector('.ts-tape');
    if (!tape) return;

    const now = new Date();
    const timeStr = [
      now.getHours().toString().padStart(2, '0'),
      now.getMinutes().toString().padStart(2, '0'),
      now.getSeconds().toString().padStart(2, '0'),
    ].join(':') + '.' + now.getMilliseconds().toString().padStart(3, '0');

    const isBuy = Math.random() > 0.5;
    const size = Math.floor(1 + Math.random() * 50);
    const counterparty = COUNTERPARTIES[Math.floor(Math.random() * COUNTERPARTIES.length)];
    const execPrice = Math.round((price + (Math.random() - 0.5) * 0.30) * 100) / 100;

    const row = document.createElement('div');
    row.className = `ts-row ${isBuy ? 'ts-buy' : 'ts-sell'}`;
    row.innerHTML =
      `<span class="ts-time">${timeStr}</span>` +
      `<span class="ts-price">${execPrice.toFixed(2)}</span>` +
      `<span class="ts-size">${size}</span>` +
      `<span class="ts-side">${isBuy ? 'BUY' : 'SELL'}</span>` +
      `<span class="ts-cpty">${counterparty}</span>`;

    tape.prepend(row);

    // Keep tape at max 200 entries
    while (tape.children.length > 200) {
      tape.removeChild(tape.lastChild);
    }

    // Flash the new row
    flashCell(row, isBuy);
  };

  // --- Spot display update ---
  const updateSpotDisplay = (newSpot, prevSpot) => {
    const el = document.querySelector('.spot-price') || document.getElementById('spot-price');
    if (el) {
      el.textContent = newSpot.toFixed(2);
      flashCell(el, newSpot >= prevSpot);
    }

    const deltaEl = document.querySelector('.spot-delta') || document.getElementById('spot-delta');
    if (deltaEl) {
      const delta = newSpot - prevSpot;
      const sign = delta >= 0 ? '+' : '';
      deltaEl.textContent = `${sign}${delta.toFixed(2)}`;
      deltaEl.className = `spot-delta ${delta >= 0 ? 'delta-up' : 'delta-down'}`;
      flashCell(deltaEl, delta >= 0);
    }
  };

  // --- Main tick loop ---
  const onTick = () => {
    const prevSpot = spot;
    const newSpot = drift();

    updateSpotDisplay(newSpot, prevSpot);
    updateOrderBook(newSpot);

    // ~60% chance of execution per tick
    if (Math.random() < 0.6) {
      appendExecution(newSpot);
    }

    // Audio feedback
    if (typeof QUANT_AUDIO !== 'undefined') {
      QUANT_AUDIO.tick(newSpot >= prevSpot ? 'up' : 'down');
    }
  };

  const start = () => {
    stop();
    const interval = regime === 'CALM' ? BASE_TICK_MS : SHOCK_TICK_MS;
    tickerInterval = setInterval(onTick, interval);
  };

  const stop = () => {
    if (tickerInterval !== null) {
      clearInterval(tickerInterval);
      tickerInterval = null;
    }
  };

  // --- Regime toggle ---
    if (badge) {
      badge.textContent = regime;
    }
  };

  return { start, stop, toggleRegime };
})();
```