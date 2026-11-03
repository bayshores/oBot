/*
 * openbot avatar engine (avatar.js)
 *
 * Draws openbot's bot characters: one round ball with two eyes, in many skins.
 *   - Solid skins: one colour (violet, coral, ...).
 *   - Pattern skins: zebra, tiger, galaxy, ... The pattern stays still and the ball
 *     is a window that moves over it, so when the ball hops or rolls it reveals a
 *     different part of the pattern (like camouflage) instead of the pattern spinning.
 *
 * Each state (idle, working, thinking, ...) has a playlist of moves. The ball picks a
 * random move from the playlist, plays it, then picks another one, blending smoothly
 * between them.
 *
 * Plain browser JavaScript. No dependencies, no build step. Load it with
 * <script src="/avatar.js"></script> before app.js. It defines one global:
 *
 *   window.OBAvatar = {
 *     SHAPES, PICKER_SHAPES, COLORS, STATES,
 *     defaults(botId)            -> { shape, color }
 *     svg(opts)                  -> string   (still image)
 *     svgUrl(opts)               -> data: URL
 *     mount(el, opts)            -> controller (live, animated)
 *     group(members, size, opts) -> string   (overlapping balls)
 *     badge(kind, opts)          -> string
 *     ink(color)                 -> '#rrggbb' (one colour that represents a skin)
 *     hue(color)                 -> tint name for the chat colours, or null
 *     normalize({shape, color})  -> { shape, color } (old saved values -> current ids)
 *   }
 *
 * A bot's look is stored as { shape: 'ball', color: <skin id> }.
 */
(function () {
  'use strict';

  const NS = 'http://www.w3.org/2000/svg';
  const TAU = Math.PI * 2;
  const CX = 50, CY = 50, R = 42; // the ball, in a 100 x 100 box

  const el = (tag, attrs, parent) => {
    const e = document.createElementNS(NS, tag);
    for (const k in attrs) e.setAttribute(k, attrs[k]);
    if (parent) parent.appendChild(e);
    return e;
  };
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  // seeded random, so every copy of a skin looks the same
  const rng = (seed) => () => { seed = (seed * 1664525 + 1013904223) >>> 0; return seed / 4294967296; };
  let uid = 0;
  const nextId = (p) => `${p}${(uid++).toString(36)}`;

  // ===========================================================================
  // Skins
  // ===========================================================================

  // id, label, representative colour (swatches, fallbacks), chat tint name
  const SOLIDS = [
    ['violet', 'Violet', '#8b5cf6', 'purple'], ['coral', 'Coral', '#ff6b57', 'red'], ['amber', 'Amber', '#f5a524', 'orange'],
    ['lime', 'Lime', '#8fd14f', 'green'], ['mint', 'Mint', '#2ec4a6', 'cyan'], ['sky', 'Sky', '#38bdf8', 'blue'],
    ['cobalt', 'Cobalt', '#4f6bff', 'blue'], ['rose', 'Rose', '#f05c9c', 'magenta'], ['slate', 'Slate', '#8a94a6', null],
    ['ink', 'Ink', '#2b2b33', null], ['sand', 'Sand', '#d9c3a0', 'brown'], ['snow', 'Snow', '#f2f0ec', null],
  ];
  const PATTERN_INFO = [
    ['zebra', 'Zebra', '#f6f4ef', null], ['tiger', 'Tiger', '#f28a2e', 'orange'], ['cow', 'Cow', '#f7f5f0', null],
    ['dots', 'Dots', '#ff5d7d', 'magenta'], ['checker', 'Checker', '#9d86f4', 'purple'], ['sunset', 'Sunset', '#ff7a6b', 'orange'],
    ['ocean', 'Ocean', '#3aa0ff', 'blue'], ['aurora', 'Aurora', '#38bdf8', 'cyan'], ['holo', 'Holo', '#c7d2ff', 'purple'],
    ['galaxy', 'Galaxy', '#2a2170', 'purple'], ['marble', 'Marble', '#e6e1d8', null], ['camo', 'Camo', '#7a8a52', 'green'],
  ];
  const SKIN = {};
  for (const [id, label, ink, hue] of SOLIDS) SKIN[id] = { id, label, ink, hue, kind: 'solid' };
  for (const [id, label, ink, hue] of PATTERN_INFO) SKIN[id] = { id, label, ink, hue, kind: 'pattern' };

  // Wavy band (one zebra stripe) as a filled path.
  function band(x0, amp, freq, phase, w0, taper, tilt) {
    const left = [], right = [];
    for (let y = -40; y <= 140; y += 4) {
      const t = (y + 40) / 180;
      const w = w0 * (taper ? Math.sin(Math.PI * clamp(t, 0, 1)) ** 0.6 : 1);
      const x = x0 + Math.sin(y * freq + phase) * amp + (y - 50) * tilt;
      left.push(`${(x - w).toFixed(2)},${y}`);
      right.unshift(`${(x + w).toFixed(2)},${y}`);
    }
    return `M${left.join('L')}L${right.join('L')}Z`;
  }
  // Soft random blob (cow spots, camo patches).
  function blob(rand, cx, cy, r) {
    const n = 7, pts = [];
    for (let i = 0; i < n; i++) { const a = (i / n) * TAU, k = r * (0.7 + rand() * 0.5); pts.push([cx + Math.cos(a) * k, cy + Math.sin(a) * k]); }
    let d = '';
    for (let i = 0; i < n; i++) {
      const p0 = pts[(i - 1 + n) % n], p1 = pts[i], p2 = pts[(i + 1) % n], p3 = pts[(i + 2) % n];
      if (!i) d += `M${p1[0].toFixed(2)},${p1[1].toFixed(2)}`;
      d += `C${(p1[0] + (p2[0] - p0[0]) / 6).toFixed(2)},${(p1[1] + (p2[1] - p0[1]) / 6).toFixed(2)} ${(p2[0] - (p3[0] - p1[0]) / 6).toFixed(2)},${(p2[1] - (p3[1] - p1[1]) / 6).toFixed(2)} ${p2[0].toFixed(2)},${p2[1].toFixed(2)}`;
    }
    return d + 'Z';
  }
  const bg = (g, fill) => el('rect', { x: -40, y: -40, width: 180, height: 180, fill }, g);
  const gradient = (defs, tag, id, attrs, stops) => {
    const gr = el(tag, Object.assign({ id, gradientUnits: 'userSpaceOnUse' }, attrs), defs);
    for (const [o, c] of stops) el('stop', { offset: o, 'stop-color': c }, gr);
    return `url(#${id})`;
  };

  // Each pattern draws into g (clipped to the ball). Patterns cover -40..140 so a moving ball
  // always has pattern under it. Returns { eye, ring } colours.
  const PATTERNS = {
    zebra(g) {
      bg(g, '#f6f4ef');
      const r = rng(11);
      for (let i = -4; i < 12; i++) el('path', { d: band(i * 13 + 2, 3 + r() * 3, 0.07 + r() * 0.03, r() * 6, 2.6 + r() * 2.4, true, 0.18), fill: '#16161a' }, g);
      return { eye: '#16161a', ring: '#f6f4ef' };
    },
    tiger(g) {
      bg(g, '#f28a2e');
      const r = rng(5);
      for (let i = -2; i < 9; i++) {
        const y = 14 + i * 14 + r() * 4;
        el('path', { d: `M-2,${y} Q16,${y - 4} ${24 + r() * 10},${y + 2} Q12,${y + 5} -2,${y + 6}Z`, fill: '#1e1410' }, g);
        el('path', { d: `M102,${y + 6} Q84,${y + 2} ${76 - r() * 10},${y + 7} Q88,${y + 11} 102,${y + 12}Z`, fill: '#1e1410' }, g);
      }
      el('ellipse', { cx: 50, cy: 78, rx: 26, ry: 14, fill: '#fbe7cf' }, g);
      return { eye: '#1e1410' };
    },
    cow(g) {
      bg(g, '#f7f5f0');
      const r = rng(23);
      for (const [x, y, s] of [[16, 22, 14], [74, 18, 12], [86, 66, 15], [30, 80, 13], [56, 40, 7], [-14, 50, 12], [112, 30, 13], [60, -18, 12], [8, 118, 12], [96, 112, 11], [-20, -10, 10], [120, 90, 10]]) {
        el('path', { d: blob(r, x, y, s), fill: '#1b1b1f' }, g);
      }
      return { eye: '#1b1b1f', ring: '#f7f5f0' };
    },
    dots(g) {
      bg(g, '#ff5d7d');
      for (let y = -36, row = 0; y < 140; y += 14, row++) for (let x = row % 2 ? -29 : -36; x < 140; x += 14) el('circle', { cx: x, cy: y, r: 3.4, fill: '#fff', opacity: 0.9 }, g);
      return { eye: '#1b1420', ring: '#ff5d7d' };
    },
    checker(g) {
      bg(g, '#ece8ff');
      for (let y = -37.5; y < 140; y += 12.5) for (let x = -37.5; x < 140; x += 12.5) {
        if (((((x + y) / 12.5) % 2) + 2) % 2 === 1) el('rect', { x, y, width: 12.5, height: 12.5, fill: '#7c5cf0', opacity: 0.55 }, g);
      }
      return { eye: '#1d1530', ring: '#ece8ff' };
    },
    sunset(g, defs, id) {
      bg(g, gradient(defs, 'linearGradient', id + 'ss', { x1: 50, y1: 8, x2: 50, y2: 92 }, [['0', '#ffd36e'], ['.45', '#ff7a6b'], ['1', '#8e5cf7']]));
      return { eye: '#22142a' };
    },
    ocean(g, defs, id) {
      bg(g, gradient(defs, 'linearGradient', id + 'oc', { x1: 40, y1: 8, x2: 64, y2: 92 }, [['0', '#7ef0ff'], ['.5', '#3aa0ff'], ['1', '#2a48d8']]));
      for (let i = -4; i < 5; i++) {
        const y = 62 + i * 11;
        el('path', { d: `M-35,${y} Q-18,${y - 5} -5,${y} T25,${y} T55,${y} T85,${y} T115,${y} T145,${y}`, fill: 'none', stroke: '#fff', 'stroke-opacity': 0.22 - Math.abs(i) * 0.03, 'stroke-width': 2.4 }, g);
      }
      return { eye: '#0d1b3a' };
    },
    aurora(g, defs, id) {
      bg(g, gradient(defs, 'linearGradient', id + 'au', { x1: 8, y1: 92, x2: 92, y2: 8 }, [['0', '#2dd4bf'], ['.5', '#38bdf8'], ['1', '#c084fc']]));
      return { eye: '#10202a' };
    },
    holo(g, defs, id) {
      bg(g, gradient(defs, 'linearGradient', id + 'ho', { x1: 8, y1: 8, x2: 92, y2: 92 }, [['0', '#ffd1f2'], ['.25', '#c7d2ff'], ['.5', '#b8fff0'], ['.75', '#fff3b0'], ['1', '#ffc4d6']]));
      el('path', { d: 'M10,-5 L32,-5 L-5,40 L-5,18Z', fill: '#fff', opacity: 0.55 }, g);
      el('path', { d: 'M40,-5 L48,-5 L-5,62 L-5,52Z', fill: '#fff', opacity: 0.4 }, g);
      return { eye: '#2a2340' };
    },
    galaxy(g, defs, id) {
      bg(g, gradient(defs, 'radialGradient', id + 'ga', { cx: 35, cy: 30, r: 72 }, [['0', '#4c3bb8'], ['.6', '#1b1648'], ['1', '#0b0a1e']]));
      el('ellipse', { cx: 68, cy: 70, rx: 26, ry: 12, fill: '#ff6ec7', opacity: 0.22, transform: 'rotate(-25 68 70)' }, g);
      const r = rng(7);
      for (let i = 0; i < 60; i++) el('circle', { cx: r() * 180 - 40, cy: r() * 180 - 40, r: 0.4 + r() * 0.9, fill: '#fff', opacity: 0.4 + r() * 0.6 }, g);
      return { eye: '#f4f1ff' };
    },
    marble(g) {
      bg(g, '#f3f0ea');
      const r = rng(31);
      for (let i = -2; i < 7; i++) {
        const y = 10 + i * 20 + r() * 8;
        el('path', { d: `M-40,${y + 6} C-15,${y - 6} 20,${y - 14 + r() * 8} 40,${y + 16} S95,${y - 10} 140,${y + 4}`, fill: 'none', stroke: '#9b958c', 'stroke-width': 0.6 + r() * 1.4, opacity: 0.55 }, g);
      }
      el('path', { d: 'M20,-5 C30,30 58,40 70,105', fill: 'none', stroke: '#c9a76a', 'stroke-width': 1.2, opacity: 0.7 }, g);
      return { eye: '#26231f' };
    },
    camo(g) {
      bg(g, '#7a8a52');
      const r = rng(41);
      const cols = ['#4f5b33', '#a9a06a', '#3a3324', '#5f6e3c'];
      for (let i = 0; i < 44; i++) el('path', { d: blob(r, r() * 180 - 40, r() * 180 - 40, 8 + r() * 9), fill: cols[i % 4] }, g);
      return { eye: '#17150f', ring: '#c9c38f' };
    },
  };

  // ===========================================================================
  // Saved values -> skin ids
  // ===========================================================================

  // Colour names saved by older builds of openbot.
  const LEGACY = {
    black: 'ink', white: 'snow', brown: 'sand', red: 'coral', orange: 'amber', yellow: 'amber', green: 'lime', cyan: 'mint',
    teal: 'mint', blue: 'cobalt', purple: 'violet', magenta: 'rose', pink: 'rose', gray: 'slate', grey: 'slate',
  };
  const hexRgb = (hex) => {
    const m = /^#?([0-9a-f]{6})$/i.exec(hex || '');
    if (!m) return null;
    const n = parseInt(m[1], 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  };
  function normColor(c) {
    const v = String(c || '').toLowerCase().trim();
    if (SKIN[v]) return v;
    if (LEGACY[v]) return LEGACY[v];
    const rgb = hexRgb(v);
    if (!rgb) return 'violet';
    let best = 'violet', bestD = Infinity;
    for (const [id, , ink] of SOLIDS) {
      const [r, g, b] = hexRgb(ink);
      const d = (r - rgb[0]) ** 2 + (g - rgb[1]) ** 2 + (b - rgb[2]) ** 2;
      if (d < bestD) { bestD = d; best = id; }
    }
    return best;
  }
  const isSkin = (c) => {
    const v = String(c || '').toLowerCase().trim();
    return !!(SKIN[v] || LEGACY[v] || hexRgb(v));
  };
  function fnv1a(s) {
    let h = 2166136261;
    for (const ch of String(s)) { h ^= ch.charCodeAt(0); h = Math.imul(h, 16777619); }
    return h >>> 0;
  }
  // New bots get a solid colour picked from their id (ink and snow are left for people to choose).
  const DEFAULT_POOL = ['violet', 'coral', 'amber', 'lime', 'mint', 'sky', 'cobalt', 'rose'];
  const defaults = (botId) => ({ shape: 'ball', color: DEFAULT_POOL[fnv1a(botId || '') % DEFAULT_POOL.length] });

  // ===========================================================================
  // Drawing one ball
  // ===========================================================================

  /**
   * Build the ball inside a new <svg>. Returns the parts the animator moves.
   * The skin group is NOT transformed: only its clip circle moves, so patterns stay put.
   */
  function build(skinId, size, opts = {}) {
    const id = nextId('oba');
    const svg = el('svg', { xmlns: NS, viewBox: '0 0 100 100', width: size, height: size, 'aria-hidden': 'true' });
    svg.style.overflow = 'visible';
    svg.style.display = 'block';
    const defs = el('defs', {}, svg);
    const clip = el('clipPath', { id: id + 'k' }, defs);
    const clipCircle = el('circle', { cx: CX, cy: CY, r: R }, clip);
    const hi = el('radialGradient', { id: id + 'h', cx: '32%', cy: '24%', r: '78%' }, defs);
    el('stop', { offset: '0', 'stop-color': '#fff', 'stop-opacity': 0.16 }, hi);
    el('stop', { offset: '.6', 'stop-color': '#fff', 'stop-opacity': 0 }, hi);
    const lo = el('radialGradient', { id: id + 'l', cx: '50%', cy: '42%', r: '62%' }, defs);
    el('stop', { offset: '.72', 'stop-color': '#000', 'stop-opacity': 0 }, lo);
    el('stop', { offset: '1', 'stop-color': '#000', 'stop-opacity': 0.16 }, lo);

    // Everything goes in `root`, so a badge can cut a hole through all of it.
    let root = svg;
    if (opts.badge) {
      const b = badgeGeometry(opts.badge);
      const mask = el('mask', { id: id + 'm', maskUnits: 'userSpaceOnUse', x: -60, y: -60, width: 220, height: 220 }, defs);
      el('rect', { x: -60, y: -60, width: 220, height: 220, fill: '#fff' }, mask);
      el('circle', { cx: b.cx, cy: b.cy, r: b.r + b.cutout, fill: '#000' }, mask);
      root = el('g', { mask: `url(#${id}m)` }, svg);
    }

    const shadow = opts.shadow === false ? null : el('ellipse', { cx: 50, cy: 97, rx: 30, ry: 3.4, fill: '#000', opacity: 0.5 }, root);
    const skinG = el('g', { 'clip-path': `url(#${id}k)` }, root);
    const mover = el('g', {}, root);
    const body = el('g', {}, mover);
    const skin = SKIN[skinId] || SKIN.violet;
    let style = { eye: '#17131f' };
    if (skin.kind === 'pattern') style = PATTERNS[skin.id](skinG, defs, id) || style;
    else {
      bg(skinG, skin.ink);
      if (skin.id === 'ink') style = { eye: '#f1eff7' };
    }
    el('circle', { cx: CX, cy: CY, r: R, fill: `url(#${id}h)` }, body);
    el('circle', { cx: CX, cy: CY, r: R, fill: `url(#${id}l)` }, body);
    el('circle', { cx: CX, cy: CY, r: R - 0.6, fill: 'none', stroke: '#fff', 'stroke-opacity': 0.1, 'stroke-width': 1.2 }, body);
    const face = el('g', {}, body);
    // Patterned skins get a soft ring around the eyes so they always read.
    const rings = style.ring ? [0, 1].map(() => el('path', { fill: 'none', stroke: style.ring, 'stroke-width': 3.2, opacity: 0.9 }, face)) : null;
    const eyes = [0, 1].map(() => el('path', { fill: style.eye, stroke: style.eye, 'stroke-linecap': 'round', 'stroke-linejoin': 'round' }, face));
    const zz = el('text', { x: 80, y: 16, 'font-size': 12, 'font-weight': 700, fill: '#cfcfd6', opacity: 0, 'font-family': 'system-ui, -apple-system, sans-serif' }, root);
    zz.textContent = 'z';
    if (opts.badge) {
      const b = badgeGeometry(opts.badge);
      const bgEl = el('g', { transform: `translate(${b.cx} ${b.cy})` }, svg);
      bgEl.innerHTML = badgeInner(opts.badge, b);
    }
    return { id, svg, clipCircle, shadow, mover, body, eyes, rings, zz };
  }

  // A pose: where the ball is and what the eyes are doing.
  const NEUTRAL = () => ({ x: 0, y: 0, rot: 0, sx: 1, sy: 1, lx: 0, ly: 0, open: 1, eye: 'open' });

  function drawEye(e, ring, x, y, w, h, mode, open) {
    let d, filled = true;
    if (mode === 'happy') { d = `M${x - w},${y + 2} Q${x},${y - h * 1.15} ${x + w},${y + 2}`; filled = false; }
    else if (mode === 'closed' || open < 0.16) { d = `M${x - w},${y + 1} Q${x},${y + 3.2} ${x + w},${y + 1}`; filled = false; }
    else {
      const hh = Math.max(w * 1.02, h * open), r = w;
      d = `M${x - w},${y - hh + r} A${r},${r} 0 0 1 ${x + w},${y - hh + r} L${x + w},${y + hh - r} A${r},${r} 0 0 1 ${x - w},${y + hh - r}Z`;
    }
    e.setAttribute('d', d);
    e.setAttribute('fill', filled ? e.getAttribute('stroke') : 'none');
    e.setAttribute('stroke-width', filled ? 0 : 3.4);
    if (ring) { ring.setAttribute('d', d); ring.setAttribute('stroke-width', filled ? 3.2 : 6.4); }
  }

  /** Put the ball in pose P with the eyes looking toward gaze [x, y] (-1..1, y down). */
  function applyPose(c, P, gaze, open) {
    const t = `translate(${P.x.toFixed(2)} ${P.y.toFixed(2)})`;
    c.mover.setAttribute('transform', t);
    const bodyT = `translate(50 92) scale(${P.sx.toFixed(3)} ${P.sy.toFixed(3)}) translate(-50 -92) rotate(${P.rot.toFixed(2)} 50 50)`;
    c.body.setAttribute('transform', bodyT);
    c.clipCircle.setAttribute('transform', `${t} ${bodyT}`);
    if (c.shadow) {
      const k = 1 - Math.min(0.5, Math.max(0, -P.y) / 40);
      c.shadow.setAttribute('cx', (50 + P.x).toFixed(2));
      c.shadow.setAttribute('rx', (30 * k * P.sx).toFixed(2));
      c.shadow.setAttribute('opacity', (0.5 * k).toFixed(3));
    }
    const wide = P.eye === 'wide' ? 1.12 : 1;
    const ex = 50 + gaze[0] * 9, ey = 48 + gaze[1] * 7;
    drawEye(c.eyes[0], c.rings && c.rings[0], ex - 12, ey, 5.2 * wide, 11 * wide, P.eye, open);
    drawEye(c.eyes[1], c.rings && c.rings[1], ex + 12, ey, 5.2 * wide, 11 * wide, P.eye, open);
  }

  // ===========================================================================
  // Moves and playlists
  // ===========================================================================

  const ease = {
    out: (t) => 1 - (1 - t) ** 3,
    inOut: (t) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2),
    back: (t) => { const c = 1.7; return 1 + (c + 1) * (t - 1) ** 3 + c * (t - 1) ** 2; },
  };
  const bell = (p) => Math.sin(Math.PI * clamp(p, 0, 1));
  // A jump: crouch first, stretch in the air, squash on landing.
  function jump(P, p, height, a = 0.18, land = 0.82) {
    if (p < a) { const q = ease.out(p / a); P.sx += 0.14 * q; P.sy -= 0.14 * q; }
    else if (p < land) { const q = (p - a) / (land - a); P.y -= height * Math.sin(Math.PI * q); const st = Math.abs(Math.cos(Math.PI * q)); P.sx -= 0.08 * st; P.sy += 0.1 * st; }
    else { const q = (p - land) / (1 - land); const s = Math.sin(Math.PI * q) * 0.13; P.sx += s; P.sy -= s; }
  }
  // Every move is f(pose, progress 0..1, direction ±1); d = length in seconds.
  const MOVES = {
    // working
    type: { d: 1.4, f(P, p) { const b = Math.abs(Math.sin(p * Math.PI * 7)); P.y -= b * 2.4; P.sy += (b - 0.5) * 0.05; P.sx -= (b - 0.5) * 0.05; P.ly = 0.7; P.lx = Math.sin(p * Math.PI * 2) * 0.35; P.open = 0.6; } },
    scan: { d: 1.6, f(P, p) { const s = Math.sin(p * TAU); P.lx = s; P.ly = -0.1; P.rot = s * 7; P.x = s * 2.5; P.open = 0.75; } },
    roll: { d: 1.5, f(P, p, dir) { const x = dir * 16 * bell(p); P.x = x; P.rot = (x / R) * 57.3; P.ly = 0.1; P.lx = dir * 0.4 * bell(p * 2); P.sy -= 0.03 * bell(p); } },
    hop: { d: 0.9, f(P, p) { jump(P, p, 13); P.ly = -0.3; } },
    flip: { d: 1.15, f(P, p, dir) { jump(P, p, 18); const q = clamp((p - 0.2) / 0.62, 0, 1); P.rot = dir * 360 * ease.inOut(q); P.eye = p > 0.2 && p < 0.82 ? 'squint' : 'open'; } },
    pump: { d: 1.0, f(P, p) { const w = Math.sin(p * TAU * 2) * (1 - p * 0.4); P.sx += w * 0.12; P.sy -= w * 0.12; P.eye = 'squint'; P.ly = 0.2; } },
    peek: { d: 1.5, f(P, p, dir) { const b = ease.inOut(bell(p)); P.x = dir * 10 * b; P.rot = dir * 16 * b; P.lx = dir * b; P.ly = -0.2 * b; P.eye = b > 0.5 ? 'wide' : 'open'; } },
    shake: { d: 0.9, f(P, p) { P.rot = Math.sin(p * Math.PI * 9) * 10 * (1 - p); P.x = Math.sin(p * Math.PI * 9) * 2 * (1 - p); P.eye = 'squint'; } },
    jelly: { d: 1.1, f(P, p) { const w = Math.sin(p * Math.PI * 6) * (1 - p); P.sx += w * 0.1; P.sy -= w * 0.1; P.rot = w * 4; P.ly = 0.3; } },
    stomp: { d: 1.2, f(P, p) { const k = Math.floor(p * 3), q = (p * 3) % 1; P.y -= Math.sin(Math.PI * q) * 4; P.rot = (k % 2 ? 1 : -1) * 6 * Math.sin(Math.PI * q); P.ly = 0.4; P.open = 0.7; } },
    // thinking
    ponder: { d: 2.4, f(P, p) { const s = Math.sin(p * TAU); P.lx = 0.6 + s * 0.25; P.ly = -0.75; P.rot = 6 + s * 4; P.open = 0.8; P.y -= bell(p) * 2; } },
    hmm: { d: 2.0, f(P, p) { const b = ease.inOut(bell(p)); P.rot = -12 * b; P.lx = -0.55 * b; P.ly = -0.6 * b; P.eye = b > 0.4 ? 'squint' : 'open'; } },
    float: { d: 2.6, f(P, p) { P.y -= Math.sin(p * TAU) * 3 + 2; P.lx = Math.sin(p * TAU) * 0.5; P.ly = -0.5; P.sx += Math.sin(p * TAU) * 0.02; } },
    // waiting for you
    bounce: { d: 1.2, f(P, p) { jump(P, (p * 2) % 1, 6, 0.2, 0.85); P.eye = 'wide'; } },
    leanin: { d: 1.8, f(P, p) { const b = ease.inOut(bell(p)); P.sx += 0.08 * b; P.sy += 0.08 * b; P.y += 2 * b; P.eye = 'wide'; } },
    tap: { d: 1.4, f(P, p) { const q = (p * 4) % 1; P.y -= Math.max(0, Math.sin(Math.PI * q)) * 2.2; P.rot = 3; P.lx = 0.2; } },
    // done
    cheer: { d: 1.5, f(P, p) { jump(P, p, 20, 0.16, 0.84); const q = clamp((p - 0.16) / 0.68, 0, 1); P.rot = 360 * ease.inOut(q); P.eye = 'happy'; } },
    wiggle: { d: 1.3, f(P, p) { P.rot = Math.sin(p * Math.PI * 6) * 12 * bell(p); P.y -= bell(p) * 4; P.eye = 'happy'; } },
    hooray: { d: 1.2, f(P, p) { jump(P, (p * 2) % 1, 9); P.eye = 'happy'; } },
    // idle
    rest: { d: 2.6, f() {} },
    glance: { d: 2.2, f(P, p, dir) { const b = ease.inOut(bell(p)); P.lx = dir * 0.9 * b; P.ly = -0.1 * b; P.rot = dir * 3 * b; } },
    curious: { d: 1.8, f(P, p, dir) { const b = ease.back(Math.min(1, p * 2)) * (p < 0.7 ? 1 : 1 - ease.inOut((p - 0.7) / 0.3)); P.rot = dir * 14 * b; P.eye = b > 0.5 ? 'wide' : 'open'; } },
    microhop: { d: 0.8, f(P, p) { jump(P, p, 5); } },
    // sleeping
    snooze: { d: 3.2, f(P, p) { const s = Math.sin(p * TAU); P.sx += 0.03 + s * 0.02; P.sy -= 0.03 + s * 0.02; P.eye = 'closed'; P.rot = 4; } },
  };
  const PLAYLIST = {
    idle: [['rest', 4], ['glance', 2], ['curious', 1], ['microhop', 1]],
    working: [['type', 3], ['scan', 2], ['roll', 2], ['hop', 2], ['flip', 1], ['pump', 1], ['peek', 2], ['shake', 1], ['jelly', 1], ['stomp', 1]],
    thinking: [['ponder', 3], ['hmm', 2], ['float', 2]],
    waiting: [['bounce', 2], ['leanin', 2], ['tap', 2]],
    done: [['cheer', 2], ['wiggle', 2], ['hooray', 2]],
    sleeping: [['snooze', 1]],
  };
  // Small avatars (sidebar) sit still when idle, like a list should; big ones fidget.
  const QUIET_IDLE = [['rest', 1]];
  // Every activity the app sends -> one of the playlists above.
  const STATE_MAP = {
    idle: 'idle', thinking: 'thinking', waiting: 'waiting', attention: 'waiting', blocked: 'waiting',
    done: 'done', celebrate: 'done', success: 'done', sleeping: 'sleeping', offline: 'sleeping',
  };
  const playlistOf = (s) => STATE_MAP[s] || 'working';
  const POSES = { happy: 'happy', surprised: 'wide', wide: 'wide', sleepy: 'closed', closed: 'closed', squint: 'squint' };

  const blend = (a, b, w) => {
    const o = {};
    for (const k in a) o[k] = typeof a[k] === 'number' ? a[k] + (b[k] - a[k]) * w : w < 0.5 ? a[k] : b[k];
    return o;
  };

  // ===========================================================================
  // Badges (the star on the main bot)
  // ===========================================================================

  const BADGE_COLORS = { main: '#f69412', working: '#0e9f5a', attention: '#c8770c', error: '#cf2b35', unread: '#4d9bff', offline: 'rgba(245,245,245,0.5)' };
  const BADGE_ALIASES = { blocked: 'attention', 'needs-attention': 'attention', waiting: 'attention', info: 'unread', running: 'working', failed: 'error' };
  const badgeKind = (k) => BADGE_ALIASES[k] || k;
  const STAR = 'M0 -1L0.2939 -0.4045L0.9511 -0.309L0.4755 0.1545L0.5878 0.809L0 0.5L-0.5878 0.809L-0.4755 0.1545L-0.9511 -0.309L-0.2939 -0.4045Z';
  // In the 100-unit avatar box: bottom-right corner.
  function badgeGeometry(kind) {
    const k = badgeKind(kind);
    if (k === 'main') return { cx: 86, cy: 86, r: 13.4, cutout: 4.5, star: 6.7 };
    return { cx: 84, cy: 84, r: 11, cutout: 4.5 };
  }
  function badgeInner(kind, b) {
    const k = badgeKind(kind);
    if (k === 'main') {
      return `<circle r="${b.r}" fill="${BADGE_COLORS.main}"/><circle r="${b.r - 0.35}" fill="none" stroke="rgba(255,255,255,0.1)" stroke-width="0.7"/>` +
        `<path d="${STAR}" transform="scale(${b.star})" fill="#f5f5f5"/>`;
    }
    return `<circle r="${b.r}" fill="${BADGE_COLORS[k] || BADGE_COLORS.unread}"/>`;
  }
  function badge(kind, opts = {}) {
    const size = opts.avatarSize || 36, b = badgeGeometry(kind), d = (b.r * 2 * size) / 100;
    const label = { main: 'Main Bot', unread: 'Unread activity', working: 'Working', attention: 'Needs attention', error: 'Error', offline: 'Offline' }[badgeKind(kind)] || '';
    return `<svg xmlns="${NS}" width="${d.toFixed(2)}" height="${d.toFixed(2)}" viewBox="${-b.r} ${-b.r} ${2 * b.r} ${2 * b.r}" role="img" aria-label="${label}">${badgeInner(kind, b)}</svg>`;
  }

  // ===========================================================================
  // Still images
  // ===========================================================================

  /** Still avatar as an SVG string. opts: { color, size = 36, expression?, badge?, className? } */
  function svg(opts = {}) {
    const c = build(normColor(opts.color), opts.size || 36, { badge: opts.badge });
    if (opts.className) c.svg.setAttribute('class', opts.className);
    const P = NEUTRAL();
    if (opts.expression && POSES[opts.expression]) P.eye = POSES[opts.expression];
    applyPose(c, P, [0, 0], 1);
    return c.svg.outerHTML;
  }

  /** Up to 4 balls overlapping (for group chats). members: [{ color } | { initial: 'S' }] */
  function group(members, size = 36) {
    const list = (members || []).slice(0, 4);
    if (!list.length) return `<svg xmlns="${NS}" width="${size}" height="${size}"></svg>`;
    const disc = list.length === 1 ? size : list.length === 2 ? (2 * size) / 3 : (5 * size) / 9;
    const at = list.length === 1 ? [[0, 0]] : list.length === 2 ? [[0, 0], [size / 3, size / 3]]
      : list.length === 3 ? [[(2 * size) / 9, 0], [0, (4 * size) / 9], [(4 * size) / 9, (4 * size) / 9]]
      : [[0, 0], [(4 * size) / 9, 0], [0, (4 * size) / 9], [(4 * size) / 9, (4 * size) / 9]];
    let body = '';
    list.forEach((m, i) => {
      const [x, y] = at[i];
      if (m.initial != null || m.color == null) {
        const r = disc / 2, t = String(m.initial || '?').slice(0, 1).toUpperCase().replace(/[&<>"]/g, '');
        body += `<circle cx="${x + r}" cy="${y + r}" r="${r}" fill="#2b2b2b"/><text x="${x + r}" y="${y + r}" fill="#a8a8a8" font-size="${(disc * 0.42).toFixed(1)}" font-family="system-ui, -apple-system, sans-serif" font-weight="500" text-anchor="middle" dominant-baseline="central">${t}</text>`;
      } else {
        const c = build(normColor(m.color), disc, { shadow: false });
        applyPose(c, NEUTRAL(), [0, 0], 1);
        c.svg.setAttribute('x', x); c.svg.setAttribute('y', y);
        body += c.svg.outerHTML;
      }
    });
    return `<svg xmlns="${NS}" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}" aria-hidden="true" style="overflow:visible">${body}</svg>`;
  }

  // ===========================================================================
  // Live avatars
  // ===========================================================================

  /** One requestAnimationFrame loop shared by every live avatar on the page. */
  const ticker = (() => {
    const jobs = new Set();
    let raf = 0;
    const loop = (now) => {
      raf = 0;
      for (const job of [...jobs]) if (!job(now)) jobs.delete(job);
      if (jobs.size) raf = requestAnimationFrame(loop);
    };
    return {
      add(job) { jobs.add(job); if (!raf) raf = requestAnimationFrame(loop); },
      remove(job) { jobs.delete(job); if (!jobs.size && raf) { cancelAnimationFrame(raf); raf = 0; } },
    };
  })();
  const COMPACT_BELOW = 48;
  const reducedMotion = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;

  /**
   * Mount a live avatar inside `el` (its contents are replaced).
   * opts: { color, size = 36, state = 'idle', interactive = false, followPointer = false, badge }
   */
  function mount(host, opts = {}) {
    const o = {
      color: normColor(opts.color), size: opts.size || 36, state: opts.state || 'idle', badge: opts.badge || null,
      interactive: !!opts.interactive, followPointer: !!opts.followPointer, restWhenIdle: opts.restWhenIdle !== false,
    };
    const reduced = reducedMotion();
    const outer = document.createElement('span');
    outer.className = 'ob-avatar';
    outer.setAttribute('aria-hidden', 'true');
    outer.style.cssText = 'display:inline-block;position:relative;flex-shrink:0;line-height:0;vertical-align:middle';
    host.replaceChildren(outer);

    let c = null;
    const seed = Math.random() * 100;
    let move = null, last = null, prev = NEUTRAL(), pose = NEUTRAL();
    const gaze = [0, 0];
    let pointer = null, expr = null, exprUntil = 0;
    let blinkAt = performance.now() + 600 + Math.random() * 2500, blinkT = -1e9, pokeAt = -1;
    let running = false, visible = true, destroyed = false, timer = 0;

    const draw = () => {
      outer.style.width = `${o.size}px`;
      outer.style.height = `${o.size}px`;
      c = build(o.color, o.size, { badge: o.badge });
      outer.replaceChildren(c.svg);
      applyPose(c, pose, gaze, 1);
    };
    const quiet = () => o.restWhenIdle && o.size < COMPACT_BELOW && playlistOf(o.state) === 'idle';
    function pick(now) {
      const list = quiet() ? QUIET_IDLE : PLAYLIST[playlistOf(o.state)];
      let total = 0;
      for (const [, w] of list) total += w;
      let name;
      for (let tries = 0; tries < 4; tries++) {
        let r = Math.random() * total;
        for (const [n, w] of list) { r -= w; if (r <= 0) { name = n; break; } }
        if (name !== last || list.length === 1) break;
      }
      last = name;
      return { name, start: now, d: MOVES[name].d * (0.9 + Math.random() * 0.25) * 1000, dir: Math.random() < 0.5 ? -1 : 1 };
    }

    // One frame. Returns true while something is still moving.
    function step(now) {
      if (!move || now - move.start >= move.d) {
        if (move) prev = pose;
        // 'done' plays one celebration, then the bot goes back to idle.
        if (move && playlistOf(o.state) === 'done') o.state = 'idle';
        move = pick(now);
      }
      const p = Math.min(1, (now - move.start) / move.d);
      let P = NEUTRAL();
      MOVES[move.name].f(P, p, move.dir);
      const calm = quiet();
      if (!calm) { const br = Math.sin((now / 1000) * 2.2 + seed) * 0.012; P.sx -= br; P.sy += br; } // breathing
      if (pokeAt > 0) {
        const q = (now - pokeAt) / 750;
        if (q >= 1) pokeAt = -1;
        else {
          const Q = NEUTRAL();
          jump(Q, q, 12, 0.2, 0.8);
          Q.eye = q > 0.2 ? 'happy' : 'squint';
          Q.rot = Math.sin(q * Math.PI * 4) * 8 * (1 - q);
          P = blend(P, Q, Math.min(1, q * 6) * (q > 0.85 ? (1 - q) / 0.15 : 1));
        }
      }
      const w = Math.min(1, (now - move.start) / 220); // ease in from the previous move so nothing pops
      if (w < 1) P = blend(prev, P, ease.out(w));
      if (expr && now < exprUntil) P.eye = expr;
      else expr = null;
      pose = P;
      let lx = P.lx, ly = P.ly;
      const st = playlistOf(o.state);
      if (pointer && (st === 'idle' || st === 'waiting') && P.eye !== 'happy') { lx = lx * 0.3 + pointer[0] * 0.9; ly = ly * 0.3 + pointer[1] * 0.9; }
      const dx = lx - gaze[0], dy = ly - gaze[1];
      gaze[0] += dx * 0.14; gaze[1] += dy * 0.14;
      let open = P.open;
      if (P.eye === 'squint') open *= 0.55;
      if (P.eye !== 'closed' && P.eye !== 'happy' && now > blinkAt) {
        blinkT = now;
        blinkAt = now + 2000 + Math.random() * 3500 + (Math.random() < 0.2 ? -1700 : 0);
      }
      const bp = (now - blinkT) / 150;
      if (bp >= 0 && bp < 1) open *= Math.abs(1 - 2 * bp) ** 1.3;
      applyPose(c, P, gaze, open);
      const sleeping = st === 'sleeping';
      const zp = (now / 2000) % 1;
      c.zz.setAttribute('opacity', sleeping ? (bell(zp) * 0.9).toFixed(3) : 0);
      c.zz.setAttribute('transform', `translate(${(zp * 6).toFixed(2)} ${(-zp * 10).toFixed(2)})`);
      // A quiet idle avatar stops the loop once it's still, and wakes up for its next blink.
      if (calm && pokeAt < 0 && w >= 1 && !expr && bp >= 1 && Math.abs(dx) + Math.abs(dy) < 0.004) {
        clearTimeout(timer);
        timer = setTimeout(wake, Math.max(16, blinkAt - now));
        return false;
      }
      return true;
    }
    const tick = (now) => {
      if (destroyed || !running) return false;
      if (document.hidden) return true;
      if (!step(now)) { running = false; return false; }
      return true;
    };
    function wake() {
      if (destroyed || reduced || running || !visible || document.hidden) return;
      running = true;
      ticker.add(tick);
    }
    const sleep = () => { running = false; ticker.remove(tick); clearTimeout(timer); };
    const still = () => {
      const P = NEUTRAL();
      const st = playlistOf(o.state);
      if (st === 'sleeping') P.eye = 'closed';
      else if (st === 'done') P.eye = 'happy';
      if (expr) P.eye = expr;
      applyPose(c, P, [0, 0], 1);
    };

    // Pointer: hover to make it look at you, click to poke.
    const onMove = (ev) => {
      const r = outer.getBoundingClientRect();
      const span = o.followPointer ? 240 : Math.max(r.width, 1);
      const dx = (ev.clientX - (r.left + r.width / 2)) / span, dy = (ev.clientY - (r.top + r.height / 2)) / span;
      const m = Math.hypot(dx, dy), k = m > 1 ? 1 / m : 1;
      pointer = [dx * k, dy * k];
      wake();
    };
    const onLeave = () => { pointer = null; wake(); };
    const onClick = () => { if (!reduced) { pokeAt = performance.now(); wake(); } };
    const off = [];
    const listen = (t, type, fn, opt) => { t.addEventListener(type, fn, opt); off.push(() => t.removeEventListener(type, fn, opt)); };
    const bind = () => {
      while (off.length) off.pop()();
      if (o.followPointer) listen(window, 'pointermove', onMove, { passive: true });
      if (o.interactive) {
        outer.style.cursor = 'pointer';
        if (!o.followPointer) { listen(outer, 'pointermove', onMove, { passive: true }); listen(outer, 'pointerleave', onLeave); }
        listen(outer, 'click', onClick);
      } else outer.style.cursor = '';
      listen(document, 'visibilitychange', () => { if (document.hidden) sleep(); else wake(); });
    };

    draw();
    bind();
    let io = null;
    if (typeof IntersectionObserver !== 'undefined') {
      io = new IntersectionObserver((entries) => {
        visible = entries[entries.length - 1].isIntersecting;
        if (visible) wake(); else sleep();
      });
      io.observe(outer);
    }
    if (reduced) still(); else wake();

    return {
      el: outer,
      /** Change any of: color, state, size, badge, interactive, followPointer. */
      set(next = {}) {
        if (destroyed) return this;
        let redraw = false;
        if (next.color != null && normColor(next.color) !== o.color) { o.color = normColor(next.color); redraw = true; }
        if (next.size != null && next.size !== o.size) { o.size = next.size; redraw = true; }
        if (next.badge !== undefined && (next.badge || null) !== o.badge) { o.badge = next.badge || null; redraw = true; }
        if (next.restWhenIdle != null) o.restWhenIdle = !!next.restWhenIdle;
        if (next.interactive != null || next.followPointer != null) {
          if (next.interactive != null) o.interactive = !!next.interactive;
          if (next.followPointer != null) o.followPointer = !!next.followPointer;
          bind();
        }
        if (next.state != null && next.state !== o.state) {
          const changed = playlistOf(next.state) !== playlistOf(o.state);
          o.state = next.state;
          if (changed && move) { prev = pose; move = pick(performance.now()); }
        }
        if (redraw) draw();
        if (reduced) still(); else wake();
        return this;
      },
      /** One-shot reaction: a little hop with a happy face. */
      poke() { if (!reduced && !destroyed) { pokeAt = performance.now(); wake(); } return this; },
      /** Look toward x, y in -1..1 (x right, y down, like screen coordinates). lookAt(null) looks forward again. */
      lookAt(x, y) {
        if (destroyed) return this;
        pointer = x == null ? null : [clamp(x, -1, 1), clamp(y || 0, -1, 1)];
        wake();
        return this;
      },
      /** Change the face for a moment: 'happy', 'surprised', 'sleepy', 'squint'. */
      setExpression(name) {
        expr = POSES[name] || null;
        exprUntil = performance.now() + 1400;
        if (reduced) still(); else wake();
        return this;
      },
      blink() { blinkAt = performance.now(); wake(); return this; },
      get state() { return o.state; },
      destroy() {
        destroyed = true;
        sleep();
        while (off.length) off.pop()();
        if (io) io.disconnect();
        outer.remove();
      },
    };
  }

  // ===========================================================================
  // Public API
  // ===========================================================================

  const ALL_SKINS = [...SOLIDS.map((s) => s[0]), ...PATTERN_INFO.map((s) => s[0])];
  const api = {
    SHAPES: ['ball'],
    PICKER_SHAPES: ['ball'],
    /** Every skin: { id, label, ink, hue, kind: 'solid' | 'pattern' } */
    COLORS: ALL_SKINS.map((id) => ({ ...SKIN[id] })),
    STATES: Object.keys(PLAYLIST),
    defaults,
    svg,
    svgUrl: (opts) => `data:image/svg+xml,${encodeURIComponent(svg(opts))}`,
    mount,
    group,
    badge,
    ink: (color) => SKIN[normColor(color)].ink,
    hue: (color) => SKIN[normColor(color)].hue,
    label: (color) => SKIN[normColor(color)].label,
    isSkin,
    normalize: ({ color } = {}) => ({ shape: 'ball', color: normColor(color) }),
  };
  if (typeof window !== 'undefined') window.OBAvatar = api;
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
})();
