/* openbot UI. Plain JS, no build step. Talks to the openbot server API (docs/API.md).
   Bot characters come from avatar.js (window.OBAvatar). */
'use strict';
(() => {
  // ───────────────────────────── helpers ─────────────────────────────
  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

  function h(tag, attrs, ...kids) {
    const el = document.createElement(tag);
    if (attrs) {
      for (const k in attrs) {
        const v = attrs[k];
        if (v == null || v === false) continue;
        if (k === 'class') el.className = v;
        else if (k === 'style') {
          if (typeof v === 'string') el.style.cssText = v;
          else for (const sk in v) el.style.setProperty(sk.startsWith('--') ? sk : sk.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase()), v[sk]);
        } else if (k === 'html') el.innerHTML = v;
        else if (k === 'text') el.textContent = v;
        else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
        else if (k === 'value') el.value = v;
        else if (k === 'checked' || k === 'disabled' || k === 'selected' || k === 'hidden') el[k] = !!v;
        else el.setAttribute(k, v === true ? '' : String(v));
      }
    }
    appendKids(el, kids);
    return el;
  }
  function appendKids(el, kids) {
    for (const k of kids) {
      if (k == null || k === false || k === true) continue;
      if (Array.isArray(k)) appendKids(el, k);
      else if (k instanceof Node) el.appendChild(k);
      else el.appendChild(document.createTextNode(String(k)));
    }
  }
  const debounce = (fn, ms) => {
    let t;
    return (...a) => {
      clearTimeout(t);
      t = setTimeout(() => fn(...a), ms);
    };
  };
  const clamp = (n, a, b) => Math.max(a, Math.min(b, n));
  const isMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  const modKey = (e) => (isMac ? e.metaKey : e.ctrlKey);
  const MOD = isMac ? '⌘' : 'Ctrl+';
  const isEditable = (el) => !!(el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' || el.isContentEditable));

  function hostOf(url) {
    try {
      return new URL(url).host;
    } catch {
      return url || 'the computer';
    }
  }
  function fmtBytes(n) {
    if (!n && n !== 0) return '—';
    const u = ['B', 'KB', 'MB', 'GB', 'TB'];
    let i = 0;
    while (n >= 1024 && i < u.length - 1) {
      n /= 1024;
      i++;
    }
    return `${n >= 10 || i === 0 ? Math.round(n) : n.toFixed(1)} ${u[i]}`;
  }
  function fmtDuration(secs) {
    if (secs == null) return '—';
    secs = Math.max(0, Math.floor(secs));
    const d = Math.floor(secs / 86400);
    const hr = Math.floor((secs % 86400) / 3600);
    const m = Math.floor((secs % 3600) / 60);
    if (d) return `${d} day${d === 1 ? '' : 's'}${hr ? `, ${hr} hr` : ''}`;
    if (hr) return `${hr} hr${m ? ` ${m} min` : ''}`;
    if (m) return `${m} min`;
    return 'less than a minute';
  }
  function fmtResetsIn(unixSecs) {
    const diff = unixSecs - Date.now() / 1000;
    if (diff <= 0) return 'Resets soon';
    const d = Math.round(diff / 86400);
    if (diff >= 86400 * 1.5) return `Resets in ${d} days`;
    if (diff >= 86400) return 'Resets in 1 day';
    const hr = Math.round(diff / 3600);
    if (diff >= 3600) return `Resets in ${hr} hour${hr === 1 ? '' : 's'}`;
    const m = Math.max(1, Math.round(diff / 60));
    return `Resets in ${m} minute${m === 1 ? '' : 's'}`;
  }
  function fmtTime(ms) {
    try {
      return new Date(ms).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
    } catch {
      return '';
    }
  }
  const fmtUsd = (n) => (n == null ? '—' : n < 0.01 && n > 0 ? '< $0.01' : `$${(n || 0).toFixed(2)}`);
  const clock = (ms) => new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  // Centered time label: "Today 3:42 PM", "Yesterday 3:42 PM", "Tue, Nov 9 3:42 PM", "Nov 9, 2025 3:42 PM"
  function timeLabel(ms) {
    const d = new Date(ms);
    const now = new Date();
    const day0 = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
    const diffDays = Math.round((day0(now) - day0(d)) / 86400000);
    const t = clock(ms);
    if (diffDays === 0) return `Today ${t}`;
    if (diffDays === 1) return `Yesterday ${t}`;
    if (d.getFullYear() === now.getFullYear()) return `${d.toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' })} ${t}`;
    return `${d.toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' })} ${t}`;
  }
  const sameDay = (a, b) => new Date(a).toDateString() === new Date(b).toDateString();
  function fmtElapsed(ms) {
    const s = Math.floor(ms / 1000);
    if (s < 60) return `${s}s`;
    if (s < 3600) return `${Math.floor(s / 60)}m`;
    return `${Math.floor(s / 3600)}h`;
  }

  // ───────────────────────────── icons ─────────────────────────────
  const ICONS = {
    search: '<circle cx="11" cy="11" r="6.5"/><path d="M16 16l4.5 4.5"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    gridPlus: '<rect x="4" y="4" width="6.5" height="6.5" rx="1.8"/><rect x="13.5" y="4" width="6.5" height="6.5" rx="1.8"/><rect x="4" y="13.5" width="6.5" height="6.5" rx="1.8"/><path d="M16.75 13.75v6M13.75 16.75h6"/>',
    gear: '<circle cx="12" cy="12" r="3"/><path d="M19.4 13.5a7.6 7.6 0 000-3l2-1.6-2-3.4-2.4 1a7.5 7.5 0 00-2.6-1.5L14 2.5h-4l-.4 2.5A7.5 7.5 0 007 6.5l-2.4-1-2 3.4 2 1.6a7.6 7.6 0 000 3l-2 1.6 2 3.4 2.4-1a7.5 7.5 0 002.6 1.5l.4 2.5h4l.4-2.5a7.5 7.5 0 002.6-1.5l2.4 1 2-3.4z"/>',
    power: '<path d="M12 3.5v8"/><path d="M7.2 6.6a7.5 7.5 0 109.6 0"/>',
    pointer: '<path d="M5.5 3.5l13 6.2-5.6 1.8-1.8 5.6z"/><path d="M13 13l5.5 5.5"/>',
    monitor: '<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8.5 20h7M12 16v4"/>',
    shield: '<path d="M12 3l7.5 3v5.5c0 4.6-3.2 8.3-7.5 9.5-4.3-1.2-7.5-4.9-7.5-9.5V6z"/>',
    vault: '<rect x="3.5" y="3.5" width="17" height="17" rx="2.5"/><circle cx="12" cy="12" r="3.3"/><path d="M12 8.7V6.8M12 17.2v-1.9M8.7 12H6.8M17.2 12h-1.9"/>',
    bars: '<path d="M6 20v-7M12 20V5M18 20v-9"/>',
    updates: '<path d="M7 17.5a4.5 4.5 0 01-.6-8.96A6 6 0 0118 9.5a4 4 0 01-.5 7.97"/><path d="M12 11v9M8.8 17l3.2 3.2 3.2-3.2"/>',
    x: '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
    wave: '<path d="M4 10.5v3M8 7.5v9M12 4.5v15M16 7.5v9M20 10.5v3"/>',
    arrowUp: '<path d="M12 19V5.5M6 11l6-6 6 6"/>',
    arrowDown: '<path d="M12 5v13.5M6 13l6 6 6-6"/>',
    stop: '<rect x="7" y="7" width="10" height="10" rx="2.2" fill="currentColor" stroke="none"/>',
    chevronDown: '<path d="M6 9l6 6 6-6"/>',
    chevronRight: '<path d="M9.5 6l6 6-6 6"/>',
    chevronLeft: '<path d="M14.5 6l-6 6 6 6"/>',
    terminal: '<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M7 9l3 3-3 3M12.5 15H17"/>',
    file: '<path d="M14 3H7a2 2 0 00-2 2v14a2 2 0 002 2h10a2 2 0 002-2V8z"/><path d="M14 3v5h5"/>',
    pencil: '<path d="M4 20h4L19 9a2.8 2.8 0 00-4-4L4 16z"/><path d="M13.5 6.5l4 4"/>',
    globe: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.5 2.7 3.8 5.7 3.8 9s-1.3 6.3-3.8 9c-2.5-2.7-3.8-5.7-3.8-9S9.5 5.7 12 3z"/>',
    bolt: '<path d="M13 3L5 13.5h6L10 21l8-10.5h-6z"/>',
    check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
    alert: '<path d="M12 3.5L2.5 20h19z"/><path d="M12 10v4.5M12 17.3v.2"/>',
    copy: '<rect x="8.5" y="8.5" width="11.5" height="11.5" rx="2.5"/><path d="M15.5 8.5V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7.5a2 2 0 002 2h2.5"/>',
    folder: '<path d="M3.5 7a2 2 0 012-2h4l2 2.5h7a2 2 0 012 2V17a2 2 0 01-2 2h-13a2 2 0 01-2-2z"/>',
    eye: '<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="3"/>',
    eyeOff: '<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="3"/><path d="M4 4l16 16"/>',
    trash: '<path d="M4.5 7h15M10 11v6M14 11v6M6.5 7l1 12a2 2 0 002 2h5a2 2 0 002-2l1-12M9.5 7V4.5h5V7"/>',
    refresh: '<path d="M20 11a8 8 0 00-14.5-4.5L4 8M4 4v4h4M4 13a8 8 0 0014.5 4.5L20 16M20 20v-4h-4"/>',
    menu: '<path d="M4 7h16M4 12h16M4 17h16"/>',
    key: '<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M17 6l3 3M14.5 8.5l2 2"/>',
    sparkle: '<path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"/>',
    external: '<path d="M9 15L18 6M10.5 6H18v7.5"/><path d="M18 16.5V18a2 2 0 01-2 2H6a2 2 0 01-2-2V8a2 2 0 012-2h1.5"/>',
    reset: '<path d="M4 12a8 8 0 108-8 8.3 8.3 0 00-6 2.6L4 8.5"/><path d="M4 4v4.5h4.5"/>',
    mail: '<rect x="3.5" y="5.5" width="17" height="13" rx="2.5"/><path d="M4 7.5l8 5.5 8-5.5"/>',
    calendar: '<rect x="3.5" y="5" width="17" height="15" rx="2.5"/><path d="M3.5 10h17M8 3v4M16 3v4"/>',
    plug: '<path d="M9 3v4M15 3v4M7 7h10v4a5 5 0 01-10 0zM12 16v5"/>',
    link: '<path d="M10 14a4 4 0 005.7 0l3-3a4 4 0 00-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 00-5.7 0l-3 3a4 4 0 005.7 5.7l1-1"/>',
    image: '<rect x="3.5" y="4.5" width="17" height="15" rx="2.5"/><circle cx="9" cy="10" r="1.8"/><path d="M20.5 16l-5-5-9 8.5"/>',
    lock: '<rect x="5" y="11" width="14" height="10" rx="2.2"/><path d="M8 11V8a4 4 0 018 0v3"/>',
    download: '<path d="M12 4v11M7 10.5l5 5 5-5M5 19.5h14"/>',
    clock: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>',
    chart: '<path d="M4 20h16"/><path d="M7 16v-5M12 16V7M17 16v-8"/>',
    pin: '<path d="M9 4h6l-1 5 3 3v2H7v-2l3-3z"/><path d="M12 14v6"/>',
    more: '<circle cx="5.5" cy="12" r="1.2" fill="currentColor" stroke="none"/><circle cx="12" cy="12" r="1.2" fill="currentColor" stroke="none"/><circle cx="18.5" cy="12" r="1.2" fill="currentColor" stroke="none"/>',
    phone: '<rect x="7" y="3" width="10" height="18" rx="2.5"/><path d="M11 17.5h2"/>',
    star: '<path d="M12 3.6l2.6 5.3 5.8.8-4.2 4.1 1 5.8L12 16.9l-5.2 2.7 1-5.8-4.2-4.1 5.8-.8z"/>',
    play: '<path d="M7 5l12 7-12 7z"/>',
    pause: '<path d="M8 5v14M16 5v14"/>',
    pinOff: '<path d="M9 4h6l-1 5 3 3v2H7v-2l3-3z"/><path d="M12 14v6M4 4l16 16"/>',
    bell: '<path d="M6 16V11a6 6 0 0112 0v5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 004 0"/>',
    keyboard: '<rect x="3" y="6" width="18" height="12" rx="2.5"/><path d="M7 10h.01M10.5 10h.01M14 10h.01M17.5 10h.01M8 14h8"/>',
    smile: '<circle cx="12" cy="12" r="8.5"/><path d="M8.6 14.2a4.2 4.2 0 006.8 0"/><circle cx="9.3" cy="10" r=".9" fill="currentColor" stroke="none"/><circle cx="14.7" cy="10" r=".9" fill="currentColor" stroke="none"/>',
    reply: '<path d="M9.5 7L4.5 12l5 5"/><path d="M5 12h9a5.5 5.5 0 015.5 5.5V19"/>',
    users: '<circle cx="9" cy="8.5" r="3.2"/><path d="M3.5 19a5.5 5.5 0 0111 0"/><path d="M15.5 5.6a3.2 3.2 0 010 5.8M17 13.8a5.5 5.5 0 013.5 5.2"/>',
    person: '<circle cx="12" cy="8.5" r="3.5"/><path d="M5 20a7 7 0 0114 0"/>',
    brain: '<path d="M9 4.5a3 3 0 00-3 3 3 3 0 00-1.5 5.5A3 3 0 007 18a2.5 2.5 0 005 0V6.5a2 2 0 00-3-2zM15 4.5a3 3 0 013 3 3 3 0 011.5 5.5A3 3 0 0117 18a2.5 2.5 0 01-5 0"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4"/>',
    moon: '<path d="M19.5 14.5A8 8 0 019.5 4.5a8 8 0 1010 10z"/>',
    display: '<rect x="3" y="4.5" width="18" height="12" rx="2"/><path d="M9 20h6"/>',
    sidebar: '<rect x="3.5" y="4.5" width="17" height="15" rx="2.5"/><path d="M9.5 4.5v15"/>',
  };
  function icon(name, size = 18, extraClass = '', sw = 1.8) {
    const span = document.createElement('span');
    span.className = 'ico ' + extraClass;
    span.innerHTML = `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="${sw}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name] || ''}</svg>`;
    return span;
  }
  const spinner = (cls = '') => h('span', { class: 'spinner ' + cls, 'aria-label': 'Working' });

  // ───────────────────────────── avatars (avatar.js) ─────────────────────────────
  const OB = () => (window.OBAvatar && typeof window.OBAvatar.svg === 'function' ? window.OBAvatar : null);
  // A bot's look is { shape: 'ball', color: <skin id> } (skins: violet, zebra, galaxy, ...).
  const FALLBACK_SKINS = ['violet', 'coral', 'amber', 'lime', 'mint', 'sky', 'cobalt', 'rose'];
  const FALLBACK_INK = { violet: '#8b5cf6', coral: '#ff6b57', amber: '#f5a524', lime: '#8fd14f', mint: '#2ec4a6', sky: '#38bdf8', cobalt: '#4f6bff', rose: '#f05c9c' };
  function hashDefaults(id) {
    let h0 = 2166136261;
    for (const ch of String(id || '')) {
      h0 ^= ch.charCodeAt(0);
      h0 = Math.imul(h0, 16777619);
    }
    return { shape: 'ball', color: FALLBACK_SKINS[(h0 >>> 0) % FALLBACK_SKINS.length] };
  }
  function defaultsFor(id) {
    const A = OB();
    try {
      if (A && A.defaults) return A.defaults(String(id || ''));
    } catch {}
    return hashDefaults(id);
  }
  // [{ id, label, kind: 'solid' | 'pattern' }]
  const skinList = () => {
    const A = OB();
    if (A && Array.isArray(A.COLORS)) return A.COLORS;
    return FALLBACK_SKINS.map((id) => ({ id, label: id[0].toUpperCase() + id.slice(1), kind: 'solid' }));
  };
  function lookOf(bot) {
    bot = bot || {};
    const A = OB();
    if (bot.color && A && A.isSkin && A.isSkin(bot.color)) return A.normalize({ color: bot.color });
    return defaultsFor(bot.id);
  }
  // openbot is always dark (there is no light mode), so avatars are always painted for a dark background.
  const themeName = () => 'dark';
  const inkOf = (color) => {
    const A = OB();
    try {
      if (A && A.ink) return A.ink(color);
    } catch {}
    return FALLBACK_INK[color] || '#8a94a6';
  };

  // Activity kind (bot.activity.kind) → avatar animation state
  const ACT_STATE = {
    thinking: 'thinking', searching_web: 'searching', reading_web: 'reading', reading_file: 'reading', writing_file: 'writing',
    running_commands: 'running', browsing: 'reading', on_its_computer: 'computing', connecting: 'orbit', waiting_for_you: 'waiting',
    working: 'working', status: 'working',
  };
  function botState(b) {
    if (!b || !b.running) return 'idle';
    if (b.activity && ACT_STATE[b.activity.kind]) return ACT_STATE[b.activity.kind];
    return 'working';
  }

  const liveAvatars = new Set();
  // Draws a bot character into `el`. live=true mounts an animated controller.
  function paintAvatar(el, look, size, { live = false, state = 'idle', interactive = false, badge = null } = {}) {
    if (el._av) {
      try {
        el._av.destroy();
      } catch {}
      el._av = null;
      liveAvatars.delete(el);
    }
    el.textContent = '';
    el._look = look.shape + '/' + look.color + '/' + (badge || '');
    el._live = live;
    const A = OB();
    const theme = themeName();
    if (A) {
      try {
        if (live && A.mount) {
          el._av = A.mount(el, Object.assign({ shape: look.shape, color: look.color, size, theme, interactive, state }, badge ? { badge } : {}));
          el._state = state;
          if (state !== 'idle') setAvatarState(el, state, true);
          liveAvatars.add(el);
          return;
        }
        el.innerHTML = A.svg(Object.assign({ shape: look.shape, color: look.color, size, theme }, badge ? { badge } : {}));
        return;
      } catch (e) {
        console.warn('avatar failed', e);
        el.textContent = '';
      }
    }
    el.append(h('span', { class: 'av-fallback', style: { display: 'block', width: '100%', height: '100%', background: inkOf(look.color) } }));
  }
  function avatar(bot, size = 36, opts = {}) {
    const el = h('span', { class: 'av', style: { width: size + 'px', height: size + 'px' } });
    el._size = size;
    if (isGroup(bot)) paintGroup(el, bot, size);
    else paintAvatar(el, lookOf(bot), size, opts);
    return el;
  }
  // A group's picture: its first Bots, overlapped.
  const groupKey = (g) => 'group:' + (g.members || []).filter((id) => S.bots.has(id)).slice(0, 3).map((id) => lookOf(S.bots.get(id)).color).join(',');
  function paintGroup(el, g, size) {
    if (el._av) {
      try {
        el._av.destroy();
      } catch {}
      el._av = null;
      liveAvatars.delete(el);
    }
    el.textContent = '';
    el._look = groupKey(g);
    el._live = false;
    el.classList.add('av-group');
    const ids = (g.members || []).filter((id) => S.bots.has(id)).slice(0, 3);
    const n = Math.max(ids.length, 1);
    const sub = n === 1 ? size : Math.round(size * (n === 2 ? 0.68 : 0.56));
    ids.forEach((id, i) => {
      const s = h('span', { class: `av-sub n${n} i${i}`, style: { width: sub + 'px', height: sub + 'px' } });
      paintAvatar(s, lookOf(S.bots.get(id)), sub);
      el.append(s);
    });
  }
  // Each look stays at least this long, so quick steps (thinking → running → thinking) don't
  // make the picture or the status text flicker.
  const HOLD_MS = 1200;
  function setAvatarState(el, state, force) {
    if (!el || !el._av) return;
    el._want = state;
    if (!force && el._state === state) return;
    const wait = (el._stateAt || 0) + HOLD_MS - Date.now();
    if (!force && wait > 0) {
      if (!el._stateT) {
        el._stateT = setTimeout(() => {
          el._stateT = null;
          if (el._av && el._want !== el._state) setAvatarState(el, el._want);
        }, wait);
      }
      return;
    }
    el._state = state;
    el._stateAt = Date.now();
    try {
      if (el._av.set) el._av.set({ state });
    } catch {}
  }
  function pokeAvatar(el) {
    try {
      if (el && el._av && el._av.poke) el._av.poke();
    } catch {}
  }
  // keep a mounted avatar in sync with a bot (repaint only when the look changes)
  function syncAvatar(el, bot, size, opts = {}) {
    if (isGroup(bot)) {
      if (el._look !== groupKey(bot)) paintGroup(el, bot, size);
      return;
    }
    el.classList.remove('av-group');
    const look = lookOf(bot);
    if (el._look !== look.shape + '/' + look.color + '/' + (opts.badge || '') || !!el._live !== !!opts.live) paintAvatar(el, look, size, opts);
    else if (opts.live) setAvatarState(el, opts.state || 'idle');
  }
  const sweepAvatars = debounce(() => {
    for (const el of liveAvatars) {
      if (!el.isConnected) {
        try {
          el._av && el._av.destroy();
        } catch {}
        el._av = null;
        liveAvatars.delete(el);
      }
    }
  }, 500);

  // ───────────────────────────── bot colour tint (shell.md 1.1c) ─────────────────────────────
  // light: fill / user-bubble (hover shade) / text · dark: fill (= user bubble) / bright / text
  const HUES = {
    red: ['#e72e3b', '#c82532', '#cd2834', '#cf2b35', '#fc2c3e', '#ff6463'],
    orange: ['#ee620d', '#d2550a', '#c14c00', '#cd5309', '#f9670e', '#fe6600'],
    yellow: ['#da830e', '#c2730b', '#aa6300', '#c8770c', '#f69412', '#e28600'],
    green: ['#10ab61', '#0d9655', '#00854a', '#0e9f5a', '#14c370', '#00ba69'],
    cyan: ['#10a693', '#0d9381', '#008273', '#0e9886', '#13b8a3', '#00b7a2'],
    blue: ['#1680f3', '#126fd4', '#0b6dd3', '#116dd2', '#1884fc', '#4d9bff'],
    purple: ['#8351e4', '#7442d1', '#7547d0', '#7845d7', '#905afb', '#a582ff'],
    magenta: ['#e52f8b', '#c82779', '#c62378', '#cf297e', '#fb3499', '#ff55a4'],
    brown: ['#9d6937', '#86592e', '#996b42', '#8c5d31', '#a16b39', '#c59e7d'],
  };
  const hueOf = (colorId) => {
    if (!colorId) return null;
    if (HUES[colorId]) return colorId;
    const A = OB();
    try {
      if (A && A.hue) return A.hue(colorId);
    } catch {}
    return null;
  };
  function hueText(colorId) {
    const hu = HUES[hueOf(colorId)];
    if (!hu) return 'var(--text-2)';
    return hu[5];
  }
  function applyTint() {
    const pref = S.prefs.accent || 'bot';
    let colorId = null;
    if (pref === 'bot') colorId = curBot() && !isGroup(curBot()) ? lookOf(curBot()).color : null;
    else if (pref !== 'none') colorId = pref;
    const hu = HUES[hueOf(colorId)];
    const st = document.documentElement.style;
    const props = ['--tint-bubble', '--tint-bubble-text', '--tint-cta', '--tint-cta-hover', '--tint-cta-text'];
    if (!hu) {
      props.forEach((p) => st.removeProperty(p));
      return;
    }
    st.setProperty('--tint-bubble', hu[3]);
    st.setProperty('--tint-bubble-text', '#f5f5f5');
    st.setProperty('--tint-cta', hu[3]);
    st.setProperty('--tint-cta-hover', hu[4]);
    st.setProperty('--tint-cta-text', '#f5f5f5');
  }

  // ───────────────────────────── constants ─────────────────────────────
  const MODEL_CHOICES = [
    ['claude-sonnet-5-5', 'Sonnet 5.5'],
    ['claude-opus-5-5', 'Opus 5.5'],
    ['claude-fable-5-1', 'Fable 5.1'],
    ['claude-haiku-5-5', 'Haiku 5.5'],
  ];
  const OLD_MODELS = { opus: 'Opus (latest)', sonnet: 'Sonnet (latest)', haiku: 'Haiku (latest)', fable: 'Fable (latest)' };
  const EFFORTS = [
    ['low', 'Low'],
    ['medium', 'Medium'],
    ['high', 'High'],
    ['xhigh', 'Extra high'],
    ['max', 'Max'],
  ];
  // Model menus. `first` is the "not set" choice; a value from an older version stays selectable.
  function modelOptions(first, current) {
    const opts = [['', first]].concat(MODEL_CHOICES);
    if (current && !opts.some(([v]) => v === current)) opts.push([current, OLD_MODELS[current] || current]);
    return opts;
  }
  const modelName = (v) => (MODEL_CHOICES.find(([m]) => m === v) || [v, OLD_MODELS[v] || v])[1];
  const TEMPLATES = [
    {
      name: 'General Bot',
      color: 'violet',
      shape: 'ball',
      blurb: 'Everyday help: questions, writing, and small tasks on the computer.',
      instructions:
        'You are a friendly, capable general assistant. Keep replies short and clear. When asked to do something on this computer, do it and report back briefly.',
    },
    {
      name: 'Research Assistant',
      color: 'galaxy',
      shape: 'ball',
      blurb: 'Digs through the web and answers with a summary and sources.',
      instructions:
        'You research topics thoroughly on the web, compare several sources, and answer with a short summary followed by the links you used.',
    },
    {
      name: 'Coding Helper',
      color: 'zebra',
      shape: 'ball',
      blurb: 'Writes, fixes, and explains code in its folder.',
      instructions:
        'You are a careful senior software engineer. Explain changes in plain language, run tests when possible, and never delete work without asking first.',
    },
    {
      name: 'Daily Planner',
      color: 'sunset',
      shape: 'ball',
      blurb: 'Turns messy to-dos into a clear plan for your day or week.',
      instructions:
        'You help plan days and weeks. Turn messy notes into prioritized task lists and short schedules. Be encouraging and concise.',
    },
  ];
  const WORK_PHRASES = ['Getting it done', 'Making progress', 'Working on it', 'Handling it', 'Chipping away'];
  const SB_MIN = 240;
  const SB_MAX = 400;
  const SB_DEFAULT = 280;
  const RAIL = 88;
  const CHAT_MIN = 424;
  const PANEL_W = 320;

  // ───────────────────────────── state ─────────────────────────────
  const S = {
    reply: null, // {botId, id, who, quote}: the message the user is replying to
    homeToken: null,
    apiBase: location.origin,
    apiToken: null,
    screensOn: new Set(), // bot ids whose screen is running
    screenStatus: null, // /api/screens/status
    screenSetup: null, // last screens.setup event
    screenUI: null, // the open screen viewer
    conn: { mode: 'local', url: '', token: '' },
    prefs: {},
    bots: new Map(),
    settings: null,
    computer: null,
    usage: null,
    version: '',
    current: null,
    msgs: new Map(),
    hasMore: new Map(),
    loading: new Set(),
    nodes: new Map(), // message id → {el, sig}
    seps: new Map(), // key → separator element
    expanded: new Set(),
    drafts: new Map(),
    pending: [],
    ws: null,
    wsGen: 0,
    wsOk: false,
    wsEverOpened: false,
    wsRetry: 1000,
    bannerTimer: null,
    settingsUI: null,
    connDraft: null,
    booted: false,
    panelOpen: false,
    hiddenOpen: false,
    runStart: new Map(),
    newDiv: null, // {botId, msgId}
    pinned: true,
    newCount: 0,
    notified: new Set(),
    lastNotify: new Map(),
    composerExpanded: false,
    initialRender: true,
  };
  const curBot = () => (S.current ? S.bots.get(S.current) : null);
  // A group chat is stored like a Bot (kind "group", with member Bot ids).
  const isGroup = (b) => !!b && b.kind === 'group';
  const realBots = () => [...S.bots.values()].filter((b) => !isGroup(b));
  // The Bot that posted a message (in a group, the one whose name is on it).
  const ownerOf = (m) => S.bots.get((m.meta && m.meta.speaker) || m.bot_id);

  // ───────────────────────────── network ─────────────────────────────
  async function req(base, token, path, { method = 'GET', body, form } = {}) {
    const headers = {};
    if (token) headers.Authorization = 'Bearer ' + token;
    let payload;
    if (form) payload = form;
    else if (body !== undefined) {
      headers['Content-Type'] = 'application/json';
      payload = JSON.stringify(body);
    }
    let res;
    try {
      res = await fetch(base + path, { method, headers, body: payload });
    } catch (e) {
      const err = new Error(`Can’t reach ${hostOf(base)}`);
      err.network = true;
      throw err;
    }
    const text = await res.text();
    let data = null;
    if (text) {
      try {
        data = JSON.parse(text);
      } catch {
        data = text;
      }
    }
    if (!res.ok) {
      const err = new Error((data && data.error) || (typeof data === 'string' && data) || `Request failed (${res.status})`);
      err.status = res.status;
      throw err;
    }
    return data;
  }
  const home = (p, o) => req(location.origin, S.homeToken, p, o);
  const api = (p, o) => req(S.apiBase, S.apiToken, p, o);
  function fileUrl(rel) {
    if (!rel) return '';
    if (/^(blob:|data:)/.test(rel)) return rel;
    const abs = /^https?:/.test(rel) ? rel : S.apiBase + rel;
    // A file-only key, not the token (an <img> can't send a header; see file_key in api.rs).
    return abs + (abs.includes('?') ? '&' : '?') + 'fk=' + encodeURIComponent(S.fileKey || '');
  }

  function initToken() {
    // A phone opened the pairing link (QR code): https://<computer>/#pair=123456. The code stays in
    // this browser (the part after "#" is never sent) and is used once by pairScreen().
    const bm = /(?:^#|&)bot=([\w-]{1,64})(?:&|$)/.exec(location.hash);
    if (bm) {
      S.openBot = bm[1];
      history.replaceState(null, '', location.pathname + location.search);
    }
    const pm = /(?:^#|&)pair=(\d{6})(?:&|$)/.exec(location.hash);
    if (pm) {
      S.pairCode = pm[1];
      history.replaceState(null, '', location.pathname + location.search);
    }
    const u = new URL(location.href);
    const t = u.searchParams.get('token');
    if (t) {
      try {
        sessionStorage.setItem('openbot.token', t);
      } catch {}
      u.searchParams.delete('token');
      history.replaceState(null, '', u.pathname + u.search + u.hash);
      S.homeToken = t;
      return;
    }
    try {
      S.homeToken = sessionStorage.getItem('openbot.token') || localStorage.getItem('openbot.token');
    } catch {}
  }

  const savePrefs = debounce(() => {
    home('/api/prefs', { method: 'PUT', body: S.prefs }).catch(() => {});
  }, 400);
  // save now (before a reboot re-reads prefs from the server)
  const savePrefsNow = () => home('/api/prefs', { method: 'PUT', body: S.prefs }).catch(() => {});

  // ───────────────────────────── desktop bridge (Tauri) ─────────────────────────────
  const TAURI = window.__TAURI_INTERNALS__;
  async function invoke(cmd, args) {
    if (!TAURI || typeof TAURI.invoke !== 'function') return null;
    try {
      return await TAURI.invoke(cmd, args || {});
    } catch {
      return null;
    }
  }
  let lastBadge = -1;
  function updateBadge() {
    if (!TAURI && !navigator.setAppBadge) return;
    let n = 0;
    for (const b of S.bots.values()) if (!b.hidden && b.unread > 0) n += b.unread;
    if (n === lastBadge) return;
    lastBadge = n;
    if (TAURI) invoke('set_badge', { count: n });
    else (n > 0 ? navigator.setAppBadge(n) : navigator.clearAppBadge()).catch(() => {});
  }
  // kind: 'done' (bot finished / sent something) or 'needs' (approval / question)
  async function maybeNotify(bot, kind, body, key) {
    if (!TAURI || !bot || bot.hidden || bot.notifications === false) return;
    if (key) {
      if (S.notified.has(key)) return;
      S.notified.add(key);
    }
    const focused = await invoke('is_focused');
    if (focused && S.current === bot.id) return;
    if (kind === 'done') {
      const last = S.lastNotify.get(bot.id) || 0;
      if (Date.now() - last < 5000) return;
    }
    S.lastNotify.set(bot.id, Date.now());
    let text = String(body || '').replace(/\s+/g, ' ').trim();
    if (text.length > 140) text = text.slice(0, 139) + '…';
    if (kind === 'done') invoke('notify', { title: bot.name, body: text || 'Open openbot to see what it did' });
    else invoke('notify', { title: `${bot.name} is waiting for you`, body: text || 'Waiting for you' });
  }

  // ───────────────────────────── toasts + tooltips ─────────────────────────────
  function toast(msg, kind = '') {
    const t = h('div', { class: 'toast ' + kind, role: 'status' }, kind === 'error' ? icon('alert', 16) : null, h('span', null, msg));
    $('#toasts').append(t);
    requestAnimationFrame(() => t.classList.add('show'));
    setTimeout(() => {
      t.classList.remove('show');
      setTimeout(() => t.remove(), 250);
    }, kind === 'error' ? 5200 : 2400);
  }
  // Errors from an unreachable or unpaired computer go to the sidebar banner, not a pile of red toasts.
  const fail = (e) => {
    if (e && (e.network || e.status === 401) && S.booted && S.apiBase !== location.origin) return noteNetworkTrouble(e.status);
    if (e && e.network && S.booted) {
      noteNetworkTrouble();
      if (S.link === 'unreachable' || Date.now() - (fail.last || 0) < 15000) return;
      fail.last = Date.now();
    }
    toast(e && e.message ? e.message : String(e), 'error');
  };

  let tipTimer = null;
  let tipFor = null;
  function hideTip() {
    clearTimeout(tipTimer);
    tipFor = null;
    $('#tip').hidden = true;
  }
  document.addEventListener('mouseover', (e) => {
    const t = e.target.closest && e.target.closest('[data-tip]');
    if (t === tipFor) return;
    hideTip();
    if (!t) return;
    tipFor = t;
    tipTimer = setTimeout(() => {
      if (!t.isConnected || tipFor !== t) return;
      const tip = $('#tip');
      tip.textContent = t.dataset.tip;
      if (t.dataset.kbd) tip.append(h('kbd', null, t.dataset.kbd));
      tip.hidden = false;
      const r = t.getBoundingClientRect();
      const tr = tip.getBoundingClientRect();
      let top = r.bottom + 6;
      if (top + tr.height > innerHeight - 4) top = r.top - tr.height - 6;
      tip.style.top = top + 'px';
      tip.style.left = clamp(r.left + r.width / 2 - tr.width / 2, 4, innerWidth - tr.width - 4) + 'px';
    }, 550);
  });
  document.addEventListener('mousedown', hideTip, true);

  // ───────────────────────────── modals / menus ─────────────────────────────
  const modalStack = [];
  function openModal(content, { cls = '', onClose, dismissable = true } = {}) {
    const overlay = h('div', { class: 'overlay ' + cls });
    overlay.append(content);
    overlay.addEventListener('mousedown', (e) => {
      if (e.target === overlay && entry.dismissable) close();
    });
    $('#layer').append(overlay);
    requestAnimationFrame(() => overlay.classList.add('open'));
    let closed = false;
    const entry = { close: () => close(), dismissable, overlay };
    function close() {
      if (closed) return;
      closed = true;
      const i = modalStack.indexOf(entry);
      if (i >= 0) modalStack.splice(i, 1);
      overlay.classList.remove('open');
      setTimeout(() => {
        overlay.remove();
        sweepAvatars();
      }, 180);
      if (onClose) onClose();
    }
    modalStack.push(entry);
    hideTip();
    return close;
  }

  function confirmDialog({ title, body, confirmText = 'Confirm', danger = false }) {
    return new Promise((resolve) => {
      let done = false;
      const finish = (v) => {
        if (done) return;
        done = true;
        resolve(v);
        close();
      };
      const okBtn = h('button', { class: 'btn ' + (danger ? 'danger-fill' : 'primary'), onclick: () => finish(true) }, confirmText);
      const box = h(
        'div',
        { class: 'dialog confirm', role: 'dialog', 'aria-modal': 'true' },
        h('h2', { class: 'dialog-title' }, title),
        body && h('p', { class: 'dialog-body' }, body),
        h('div', { class: 'dialog-actions' }, h('button', { class: 'btn', onclick: () => finish(false) }, 'Cancel'), okBtn)
      );
      const close = openModal(box, { onClose: () => finish(false) });
      setTimeout(() => okBtn.focus(), 30);
    });
  }

  let openMenuEl = null;
  function closeMenu() {
    if (openMenuEl) {
      openMenuEl.remove();
      openMenuEl = null;
    }
  }
  // items: {label, icon, action, danger, kbd, checked} | '-'
  function buildMenu(items) {
    const m = h('div', { class: 'menu', role: 'menu' });
    for (const it of items) {
      if (!it) continue;
      if (it === '-') {
        if (m.lastElementChild && !m.lastElementChild.classList.contains('menu-sep')) m.append(h('div', { class: 'menu-sep' }));
        continue;
      }
      if (it.el) {
        m.append(it.el);
        continue;
      }
      m.append(
        h(
          'button',
          {
            class: 'menu-item' + (it.danger ? ' danger' : ''),
            role: 'menuitem',
            onclick: () => {
              closeMenu();
              it.action();
            },
          },
          it.icon ? icon(it.icon, 16) : null,
          h('span', null, it.label),
          it.checked ? h('span', { class: 'kbd' }, icon('check', 14)) : it.kbd ? h('span', { class: 'kbd' }, it.kbd) : null
        )
      );
      if (it.disabled) m.lastElementChild.disabled = true;
    }
    while (m.lastElementChild && m.lastElementChild.classList.contains('menu-sep')) m.lastElementChild.remove();
    return m;
  }
  function showMenu(items, x, y) {
    closeMenu();
    const m = buildMenu(items);
    document.body.append(m);
    const r = m.getBoundingClientRect();
    m.style.left = clamp(x, 8, innerWidth - r.width - 8) + 'px';
    m.style.top = clamp(y, 8, innerHeight - r.height - 8) + 'px';
    openMenuEl = m;
    hideTip();
  }
  // placement: 'top-start' (above, left-aligned) or 'bottom-start'
  function menuAt(anchor, items, placement = 'bottom-start') {
    closeMenu();
    const m = buildMenu(items);
    document.body.append(m);
    const a = anchor.getBoundingClientRect();
    const r = m.getBoundingClientRect();
    const top = placement === 'top-start' ? a.top - r.height - 6 : a.bottom + 6;
    m.style.left = clamp(a.left, 8, innerWidth - r.width - 8) + 'px';
    m.style.top = clamp(top, 8, innerHeight - r.height - 8) + 'px';
    openMenuEl = m;
    hideTip();
  }
  document.addEventListener('mousedown', (e) => {
    if (openMenuEl && !openMenuEl.contains(e.target)) closeMenu();
  });
  window.addEventListener('blur', closeMenu);

  // ───────────────────────────── theme ─────────────────────────────
  // Always dark. Older installs may still have prefs.theme saved; it is ignored.
  function applyTheme() {
    document.documentElement.dataset.theme = 'dark';
    applyTint();
  }

  // ───────────────────────────── screens (token / offline / boot) ─────────────────────────────
  function showScreen(content) {
    const s = $('#screen');
    s.textContent = '';
    s.append(content);
    s.hidden = false;
  }
  function hideScreen() {
    $('#screen').hidden = true;
    $('#screen').textContent = '';
    sweepAvatars();
  }
  function bootScreen() {
    const remote = S.conn && S.conn.mode === 'remote' && S.conn.url;
    showScreen(h('div', { class: 'screen-card plain' }, spinner('lg'), h('div', { class: 'screen-sub' }, remote ? `Connecting to ${computerName()}…` : 'Starting openbot…')));
  }
  const onThisComputer = () => /^(localhost|127\.0\.0\.1|\[::1\])$/.test(location.hostname);
  // What a newly paired browser calls itself in Settings › Paired devices.
  function browserDeviceName() {
    const ua = navigator.userAgent || '';
    if (/iPhone/.test(ua)) return 'iPhone';
    if (/iPad/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)) return 'iPad';
    if (/Android/.test(ua)) return /Mobile/.test(ua) ? 'Android phone' : 'Android tablet';
    return 'Web browser';
  }
  // A phone (or another computer's browser) opened this computer's address: it pairs with a
  // 6-digit code and gets its own token, which can be removed later in Settings.
  function pairScreen(msg) {
    const input = h('input', { class: 'field pair-input', type: 'text', inputmode: 'numeric', autocomplete: 'one-time-code', maxlength: '7', placeholder: '123 456', 'aria-label': 'Pairing code' });
    const status = h('div', { class: 'screen-note', role: 'status' }, msg || '');
    const go = async () => {
      const code = input.value.replace(/\D/g, '');
      if (code.length !== 6) {
        status.textContent = 'Enter the 6-digit code.';
        return input.focus();
      }
      pair.disabled = true;
      status.textContent = 'Pairing…';
      try {
        const r = await req(location.origin, null, '/api/pair', { method: 'POST', body: { code, device_name: browserDeviceName() } });
        S.homeToken = r.token;
        try {
          localStorage.setItem('openbot.token', r.token);
          sessionStorage.setItem('openbot.token', r.token);
        } catch {}
        S.pairCode = null;
        boot();
      } catch (e) {
        S.pairCode = null;
        pair.disabled = false;
        status.textContent = e.message;
        input.select();
      }
    };
    const pair = h('button', { class: 'btn primary', onclick: go }, 'Pair');
    input.addEventListener('keydown', (e) => e.key === 'Enter' && go());
    input.addEventListener('input', () => {
      const d = input.value.replace(/\D/g, '').slice(0, 6);
      const v = d.length > 3 ? d.slice(0, 3) + ' ' + d.slice(3) : d;
      if (v !== input.value) input.value = v;
    });
    showScreen(
      h(
        'div',
        { class: 'screen-card' },
        h('div', { class: 'screen-art' }, avatar({ id: 'openbot', shape: 'ball', color: 'violet' }, 72, { live: true })),
        h('h1', { class: 'screen-title' }, `Pair this ${browserDeviceName() === 'Web browser' ? 'browser' : browserDeviceName()} with openbot`),
        h('p', { class: 'screen-sub' }, 'On the computer that runs your Bots, open openbot › Settings › Computer and choose Pair a new device. Scan the QR code with this phone’s camera, or type the 6-digit code here.'),
        h('div', { class: 'screen-form' }, input, pair),
        status,
        h('button', { class: 'btn ghost sm screen-alt', onclick: () => tokenScreen(null, { token: true }) }, 'Use a token instead')
      )
    );
    if (S.pairCode) {
      input.value = S.pairCode.slice(0, 3) + ' ' + S.pairCode.slice(3);
      go();
    } else setTimeout(() => input.focus(), 50);
  }
  function tokenScreen(msg, opts) {
    if (!TAURI && !onThisComputer() && !(opts && opts.token)) return pairScreen(msg && 'This device isn’t paired anymore. Pair it again with a new code.');
    const input = h('input', { class: 'field', type: 'password', placeholder: 'Paste your openbot token', autocomplete: 'off' });
    const go = () => {
      const v = input.value.trim();
      if (!v) return input.focus();
      S.homeToken = v;
      try {
        sessionStorage.setItem('openbot.token', v);
        localStorage.setItem('openbot.token', v);
      } catch {}
      boot();
    };
    input.addEventListener('keydown', (e) => e.key === 'Enter' && go());
    showScreen(
      h(
        'div',
        { class: 'screen-card' },
        h('div', { class: 'screen-art' }, avatar({ id: 'openbot', shape: 'ball', color: 'violet' }, 72, { live: true })),
        h('h1', { class: 'screen-title' }, 'Welcome to openbot'),
        h(
          'p',
          { class: 'screen-sub' },
          msg || 'This page needs your openbot token. Open openbot from the app, or paste the token that openbot-server printed when it started.'
        ),
        h('div', { class: 'screen-form' }, input, h('button', { class: 'btn primary', onclick: go }, 'Continue'))
      )
    );
    setTimeout(() => input.focus(), 50);
  }
  function offlineScreen(err) {
    const remote = S.conn.mode === 'remote';
    const name = computerName();
    const unauthorized = err && err.status === 401;
    clearTimeout(offlineScreen.t);
    const status = h('div', { class: 'screen-note' });
    const actions = h('div', { class: 'screen-actions' });
    let title;
    let sub;
    let list = null;
    if (unauthorized && remote) {
      title = `${ThisDevice()} isn’t paired with ${name} anymore`;
      sub = 'It may have been removed from that computer’s paired devices. Pair again with a new 6-digit code from that computer.';
      actions.append(
        h('button', { class: 'btn primary', onclick: () => openSetupWizard({ step: 2, url: S.conn.url }) }, 'Pair again'),
        h('button', { class: 'btn', onclick: () => useThisDevice() }, `Use ${thisDevice()} instead`)
      );
    } else if (remote) {
      title = `Can’t reach ${name}`;
      sub = 'openbot keeps trying in the background. Check that:';
      list = h('ul', { class: 'screen-list' }, h('li', null, `${name} is turned on and awake`), h('li', null, `Tailscale is on, on ${thisDevice()} and on ${name}`), h('li', null, `openbot is running on ${name}`));
      actions.append(
        h('button', { class: 'btn primary', onclick: () => boot() }, icon('refresh', 15), 'Retry'),
        h('button', { class: 'btn', onclick: () => useThisDevice() }, `Use ${thisDevice()} instead`),
        h('button', { class: 'btn', onclick: () => openSetupWizard({ step: 1 }) }, 'Help')
      );
      // keep checking with backoff (5 s → 30 s) and come back on our own
      const wait = Math.min(30, (offlineScreen.wait = offlineScreen.wait ? offlineScreen.wait * 2 : 5));
      let left = wait;
      const tick = async () => {
        if (!status.isConnected) return;
        if (left <= 0) {
          status.textContent = 'Trying again…';
          const ok = await fetch(S.conn.url.replace(/\/+$/, '') + '/api/hello').then((r) => r.ok, () => false);
          if (!status.isConnected) return;
          if (ok) return boot();
          return offlineScreen(err);
        }
        status.textContent = `Trying again in ${left} s…`;
        left--;
        offlineScreen.t = setTimeout(tick, 1000);
      };
      offlineScreen.t = setTimeout(tick, 0);
    } else {
      title = 'openbot couldn’t start';
      sub = `Its local service isn’t answering. Quit openbot and open it again. ${err && err.message ? err.message : ''}`;
      actions.append(h('button', { class: 'btn primary', onclick: () => boot() }, icon('refresh', 15), 'Try again'));
    }
    showScreen(
      h(
        'div',
        { class: 'screen-card' },
        h('div', { class: 'screen-art' }, avatar({ id: 'offline', shape: 'ball', color: 'slate' }, 72, { live: true, state: 'sleeping' })),
        h('h1', { class: 'screen-title' }, title),
        h('p', { class: 'screen-sub' + (list ? ' tight' : '') }, sub),
        list,
        actions,
        status
      )
    );
  }

  // ───────────────────────────── boot ─────────────────────────────
  async function boot() {
    closeWS();
    clearTimeout(boot.t);
    if (!S.booted) boot.t = setTimeout(() => !S.booted && $('#screen').hidden && bootScreen(), 400);
    if (!S.homeToken) return tokenScreen();
    try {
      const c = await home('/api/connection');
      S.conn = Object.assign({ mode: 'local', url: '', token: '' }, c || {});
    } catch (e) {
      if (e.status === 401) return tokenScreen('That token didn’t work. Paste the token printed by openbot-server.');
      if (e.network) return offlineScreen(e);
      S.conn = { mode: 'local', url: '', token: '' };
    }
    try {
      S.prefs = (await home('/api/prefs')) || {};
      if (typeof S.prefs !== 'object') S.prefs = {};
    } catch {
      S.prefs = S.prefs || {};
    }
    // remember, before selectBot() writes lastBotId, whether this looks like a brand-new install
    if (S.freshPrefs === undefined) S.freshPrefs = !S.prefs.onboarded && !S.prefs.lastBotId && !S.prefs.userName;
    applyTheme();
    layoutShell();
    if (S.conn.mode === 'remote' && S.conn.url) {
      S.apiBase = S.conn.url.replace(/\/+$/, '');
      S.apiToken = S.conn.token;
    } else {
      S.apiBase = location.origin;
      S.apiToken = S.homeToken;
    }
    let b;
    try {
      b = await api('/api/bootstrap');
    } catch (e) {
      S.booted = false;
      return offlineScreen(e);
    }
    clearTimeout(offlineScreen.t);
    offlineScreen.wait = 0;
    S.link = 'ok';
    S.msgs.clear();
    S.hasMore.clear();
    S.nodes.clear();
    S.seps.clear();
    applyBootstrap(b);
    hideScreen();
    S.booted = true;
    $('#app').classList.remove('booting');
    renderMe();
    renderSidebar();
    const tapped = S.openBot && S.bots.has(S.openBot) ? S.openBot : null;
    S.openBot = null;
    const want = tapped || (S.current && S.bots.has(S.current) ? S.current : S.prefs.lastBotId);
    const first = (want && S.bots.has(want) && want) || (orderedBots()[0] || {}).id;
    if (first) selectBot(first, { focus: true });
    else {
      S.current = null;
      renderChat();
    }
    connectWS();
    renderSysBanner();
    if (S.settingsUI) rerenderSettings(true);
    maybeOnboard();
    loadScreenStatus();
  }
  function applyBootstrap(b) {
    S.bots.clear();
    for (const bot of (b && b.bots) || []) {
      S.bots.set(bot.id, bot);
      trackRun(bot);
    }
    S.settings = (b && b.settings) || S.settings;
    S.computer = (b && b.computer) || S.computer;
    S.usage = (b && b.usage) || S.usage;
    S.version = (b && b.version) || S.version;
    S.fileKey = (b && b.file_key) || S.fileKey;
    migratePins();
    renderUsageBanner();
    renderSysBanner();
  }
  function trackRun(bot, prev) {
    if (bot.running && !(prev && prev.running)) {
      if (!S.runStart.has(bot.id)) S.runStart.set(bot.id, Date.now());
    } else if (!bot.running) S.runStart.delete(bot.id);
  }

  // ───────────────────────────── websocket ─────────────────────────────
  function closeWS() {
    S.wsGen++;
    if (S.ws) {
      try {
        S.ws.close();
      } catch {}
    }
    S.ws = null;
    S.wsOk = false;
    S.wsEverOpened = false;
    S.wsRetry = 1000;
    setConnBanner(false);
  }
  function connectWS() {
    const gen = ++S.wsGen;
    let u;
    try {
      u = new URL(S.apiBase);
    } catch {
      return;
    }
    u.protocol = u.protocol === 'https:' ? 'wss:' : 'ws:';
    u.pathname = '/api/events';
    let ws;
    try {
      // The token rides along as a subprotocol, not in the address.
      ws = new WebSocket(u.toString(), ['openbot', 'openbot.auth.' + (S.apiToken || '')]);
    } catch {
      return scheduleReconnect(gen);
    }
    S.ws = ws;
    ws.onopen = () => {
      if (gen !== S.wsGen) return;
      const reconnected = S.wsEverOpened;
      S.wsOk = true;
      S.wsEverOpened = true;
      S.wsRetry = 1000;
      S.wsFails = 0;
      setConnBanner(false);
      setLink('ok');
      presence.sent = null;
      sendPresence(true);
      if (reconnected) resync();
      if (S.settingsUI) rerenderSettings();
    };
    ws.onmessage = (ev) => {
      if (gen !== S.wsGen) return;
      let e;
      try {
        e = JSON.parse(ev.data);
      } catch {
        return;
      }
      try {
        onEvent(e);
      } catch (err) {
        console.error('event error', err, e);
      }
    };
    ws.onclose = () => {
      if (gen !== S.wsGen) return;
      S.wsOk = false;
      S.wsFails = (S.wsFails || 0) + 1;
      setConnBanner(true);
      scheduleReconnect(gen);
      // after two failed tries, find out why (computer off vs. this device unpaired)
      if (S.wsFails >= 2 && S.link !== 'unreachable' && S.link !== 'unpaired') noteNetworkTrouble();
      if (S.settingsUI) rerenderSettings();
    };
    ws.onerror = () => {};
  }
  // Tell the computer whether someone is using this window, so phone notifications wait while
  // you're looking (see push.rs): it's in front, focused and touched in the last 2 minutes.
  const presence = { lastInput: Date.now(), sent: null, hiddenAt: 0 };
  const isActive = () => document.visibilityState === 'visible' && document.hasFocus() && Date.now() - presence.lastInput < 120000;
  function sendPresence(force) {
    const a = isActive();
    if (!force && a === presence.sent) return;
    if (!S.ws || S.ws.readyState !== 1) return;
    try {
      S.ws.send(JSON.stringify({ type: 'presence', active: a }));
      presence.sent = a;
    } catch {}
  }
  for (const ev of ['pointerdown', 'keydown', 'touchstart', 'wheel', 'mousemove'])
    addEventListener(
      ev,
      () => {
        presence.lastInput = Date.now();
        if (presence.sent === false) sendPresence();
      },
      { passive: true, capture: true }
    );
  addEventListener('focus', () => sendPresence());
  addEventListener('blur', () => sendPresence());
  setInterval(() => sendPresence(), 30000);
  // Phones pause a web app in the background and its connection dies quietly. Coming back:
  // reconnect at once (catching up on what was missed) instead of waiting for the retry timer.
  function wake() {
    if (!S.booted) return;
    const away = presence.hiddenAt ? Date.now() - presence.hiddenAt : 0;
    presence.hiddenAt = 0;
    const ws = S.ws;
    if (!ws || ws.readyState > 1 || away > 15000) {
      S.wsGen++;
      try {
        if (ws) ws.close();
      } catch {}
      S.wsRetry = 1000;
      connectWS();
    }
    sendPresence(true);
  }
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') wake();
    else {
      presence.hiddenAt = Date.now();
      sendPresence();
    }
  });
  addEventListener('pageshow', (e) => e.persisted && wake());
  addEventListener('online', () => wake());
  function scheduleReconnect(gen) {
    setTimeout(() => {
      if (gen === S.wsGen) connectWS();
    }, S.link === 'unpaired' ? 30000 : S.wsRetry);
    S.wsRetry = Math.min(S.wsRetry * 2, 10000);
  }
  function setConnBanner(show) {
    clearTimeout(S.bannerTimer);
    const el = $('#connBanner');
    if (!show) {
      if (!el.classList.contains('bad')) el.hidden = true;
      return;
    }
    if (el.classList.contains('bad')) return;
    S.bannerTimer = setTimeout(() => {
      if (S.wsOk || el.classList.contains('bad')) return;
      el.textContent = '';
      el.append(spinner('xs'), h('span', null, `Reconnecting to ${S.conn.mode === 'remote' ? hostOf(S.apiBase) : 'openbot'}…`));
      el.hidden = false;
    }, 1500);
  }
  async function resync() {
    try {
      const b = await api('/api/bootstrap');
      applyBootstrap(b);
      const keep = S.current;
      S.msgs.clear();
      S.hasMore.clear();
      renderSidebar();
      if (keep && S.bots.has(keep)) {
        await loadMessages(keep);
        if (S.current === keep) renderMessages(true);
      } else renderChat();
      if (S.panelOpen) renderPanel();
    } catch (e) {
      console.warn('resync failed', e);
    }
  }

  function onEvent(e) {
    switch (e.type) {
      case 'hello':
        if (e.version) S.version = e.version;
        break;
      case 'resync':
        resync();
        break;
      case 'bot.created':
      case 'bot.updated': {
        if (!e.bot) break;
        const prev = S.bots.get(e.bot.id);
        S.bots.set(e.bot.id, e.bot);
        trackRun(e.bot, prev);
        renderSidebar();
        if (e.bot.id === S.current) {
          renderChatHeader();
          applyTint();
          updateComposer();
          scheduleRender();
          if (e.bot.unread > 0) markReadSoon();
          if (S.panelOpen) refreshPanelBot();
        }
        break;
      }
      case 'bot.deleted': {
        S.bots.delete(e.bot_id);
        S.msgs.delete(e.bot_id);
        if (S.prefs.pinned) S.prefs.pinned = S.prefs.pinned.filter((x) => x !== e.bot_id);
        renderSidebar();
        if (S.current === e.bot_id) {
          const next = orderedBots()[0];
          if (next) selectBot(next.id);
          else {
            S.current = null;
            setPanel(false);
            renderChat();
          }
        }
        break;
      }
      case 'message.created':
      case 'message.updated':
        if (e.message) upsertMessage(e.message, e.type === 'message.created');
        break;
      case 'screen.updated':
        if (e.running) S.screensOn.add(e.bot_id);
        else S.screensOn.delete(e.bot_id);
        renderScreenBtn();
        if (S.screenUI && S.screenUI.botId === e.bot_id && !e.running) S.screenUI.stopped();
        break;
      case 'screens.setup':
        S.screenSetup = e;
        if (e.state !== 'running') loadScreenStatus();
        if (S.settingsUI && S.settingsUI.tab === 'computer') rerenderSettings();
        if (e.state === 'done') toast('Bot screens are set up');
        if (e.state === 'error') toast('Setting up Bot screens didn’t finish', 'error');
        break;
      case 'messages.deleted': {
        const gone = new Set(e.ids || []);
        if (S.msgs.has(e.bot_id)) S.msgs.set(e.bot_id, S.msgs.get(e.bot_id).filter((x) => !gone.has(x.id)));
        if (e.bot_id === S.current) renderMessages(false);
        break;
      }
      case 'messages.cleared':
        if (S.msgs.has(e.bot_id)) S.msgs.set(e.bot_id, []);
        S.hasMore.set(e.bot_id, false);
        if (e.bot_id === S.current) renderMessages(true);
        break;
      case 'usage.updated':
        S.usage = e.usage;
        renderUsageBanner();
        if (S.settingsUI && S.settingsUI.tab === 'usage') rerenderSettings();
        break;
      case 'settings.updated':
        S.settings = e.settings;
        if (S.settingsUI) rerenderSettings();
        break;
      case 'github.connected':
        if (S.settingsUI) {
          S.settingsUI.github = null;
          rerenderSettings(true);
        }
        toast(`GitHub is connected as ${e.login}`);
        break;
      case 'github.failed':
        if (S.settingsUI && S.settingsUI.github) {
          S.settingsUI.github.err = e.error;
          rerenderSettings(true);
        }
        break;
      case 'account.connected':
        if (S.settingsUI && S.settingsUI.connect) {
          S.settingsUI.connect = null;
          rerenderSettings(true);
        }
        toast(`${e.email} is connected`);
        break;
      case 'devices.updated':
        if (S.devicesUI) {
          S.devicesUI.list = null;
          S.devicesUI.err = null;
          if (S.settingsUI && S.settingsUI.tab === 'computer') rerenderSettings();
        }
        break;
      case 'routines.updated':
        if (S.panelOpen && (!e.bot_id || e.bot_id === S.current)) loadPanelLists('routines');
        break;
      case 'memories.updated':
        if (S.panelOpen && (!e.bot_id || e.bot_id === S.current)) loadPanelLists('memories');
        break;
      default:
        break;
    }
  }

  // ───────────────────────────── shell layout (sidebar width, rail, panel) ─────────────────────────────
  const app = $('#app');
  const isPhone = () => innerWidth <= 520;
  function sidebarWidth() {
    return clamp(Number(S.prefs.sidebarWidth) || SB_DEFAULT, SB_MIN, SB_MAX);
  }
  function layoutShell() {
    const w = sidebarWidth();
    const rail = !isPhone() && (!!S.prefs.sidebarCollapsed || innerWidth < w + CHAT_MIN);
    app.classList.toggle('rail', rail);
    app.style.setProperty('--sbw', (rail ? RAIL : w) + 'px');
    const sb = rail ? RAIL : w;
    app.classList.toggle('panel-float', S.panelOpen && !isPhone() && innerWidth - sb - PANEL_W < CHAT_MIN);
    if (S.booted && (S.link && S.link !== 'ok')) renderSysBanner();
  }
  function animateShell(fn) {
    app.classList.add('sb-anim');
    fn();
    clearTimeout(animateShell.t);
    animateShell.t = setTimeout(() => app.classList.remove('sb-anim'), 260);
  }
  function toggleRail() {
    if (isPhone()) {
      app.classList.toggle('sidebar-open');
      return;
    }
    const w = sidebarWidth();
    const autoRail = innerWidth < w + CHAT_MIN;
    if (autoRail && !S.prefs.sidebarCollapsed) {
      toast('Make the window wider to show the full sidebar');
      return;
    }
    animateShell(() => {
      S.prefs.sidebarCollapsed = !S.prefs.sidebarCollapsed;
      savePrefs();
      layoutShell();
    });
  }
  window.addEventListener('resize', () => {
    layoutShell();
    closeMenu();
    layoutComposer();
  });
  // drag the sidebar edge: 240–400px; below 210px it snaps to the 88px rail, re-expands past 220px
  (() => {
    const handle = $('#sbResize');
    let drag = null;
    handle.addEventListener('pointerdown', (e) => {
      if (isPhone()) return;
      e.preventDefault();
      handle.setPointerCapture(e.pointerId);
      drag = { id: e.pointerId };
      document.body.classList.add('resizing');
    });
    handle.addEventListener('pointermove', (e) => {
      if (!drag) return;
      const x = e.clientX;
      if (S.prefs.sidebarCollapsed) {
        if (x > 220) {
          S.prefs.sidebarCollapsed = false;
          S.prefs.sidebarWidth = clamp(x, SB_MIN, SB_MAX);
        }
      } else if (x < 210) S.prefs.sidebarCollapsed = true;
      else S.prefs.sidebarWidth = clamp(x, SB_MIN, SB_MAX);
      layoutShell();
    });
    const end = () => {
      if (!drag) return;
      drag = null;
      document.body.classList.remove('resizing');
      savePrefs();
    };
    handle.addEventListener('pointerup', end);
    handle.addEventListener('pointercancel', end);
    handle.addEventListener('dblclick', () => {
      S.prefs.sidebarWidth = SB_DEFAULT;
      S.prefs.sidebarCollapsed = false;
      savePrefs();
      animateShell(layoutShell);
    });
  })();

  // ───────────────────────────── sidebar ─────────────────────────────
  const sortedBots = () => [...S.bots.values()].sort((a, b) => (b.updated_at || 0) - (a.updated_at || 0));
  const pinnedIds = () => {
    const order = Array.isArray(S.prefs.pinOrder) ? S.prefs.pinOrder : [];
    const rank = (b) => {
      const i = order.indexOf(b.id);
      return i < 0 ? 1e6 + (b.created_at || 0) / 1e13 : i;
    };
    return [...S.bots.values()].filter((b) => b.pinned && !b.hidden).sort((a, b) => rank(a) - rank(b)).map((b) => b.id);
  };
  // order used by ⌘1…⌘9, ⌥↑/↓ and "first bot": pinned tiles, then rows
  function orderedBots() {
    const pins = pinnedIds();
    const rest = sortedBots().filter((b) => !b.hidden && !pins.includes(b.id));
    return [...pins.map((id) => S.bots.get(id)), ...rest];
  }
  function renderMe() {
    const name = (S.prefs.userName || 'You').trim() || 'You';
    $('#meCircle').textContent = name[0].toUpperCase();
    $('#meName').textContent = name;
  }
  function isWaiting(b) {
    return !!(b.running && ((b.activity && b.activity.kind === 'waiting_for_you') || /^Needs your OK: /.test(b.last_preview || '')));
  }
  function previewOf(b) {
    if (isWaiting(b)) {
      const reason = /^Needs your OK: /.test(b.last_preview || '') ? b.last_preview.replace(/^Needs your OK: /, '') : (b.activity && b.activity.label) || '';
      return { warn: true, text: 'Waiting for you' + (reason && reason !== 'Waiting for you' ? ': ' + reason : '') };
    }
    const d = b.id !== S.current && S.drafts.get(b.id);
    if (d && d.text && d.text.trim()) return { draft: d.text.trim().split('\n')[0] };
    const firstLine = (b.last_preview || '').split('\n').find((l) => l.trim()) || '';
    return { text: firstLine.trim() || 'No messages yet' };
  }

  const rowCache = new Map(); // key → {el, parts}
  function botRowEl(b, layout) {
    const key = layout + ':' + b.id;
    let c = rowCache.get(key);
    if (!c) {
      const av = h('span', { class: 'av', style: { width: (layout === 'tile' ? 60 : 36) + 'px', height: (layout === 'tile' ? 60 : 36) + 'px' } });
      av._size = layout === 'tile' ? 60 : 36;
      const badge = h('span', { class: 'av-badge' });
      let el;
      const parts = { av, badge };
      const handlers = {
        onclick: (e) => {
          if (e.detail > 1) return;
          selectBot(b.id);
        },
        ondblclick: () => startRename(b.id, parts),
        oncontextmenu: (e) => {
          e.preventDefault();
          botMenu(S.bots.get(b.id), e.clientX, e.clientY);
        },
        onkeydown: (e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            selectBot(b.id);
          }
        },
        onpointermove: (e) => {
          if (av._av && av._av.lookAt) {
            const r = av.getBoundingClientRect();
            if (!r.width) return;
            const nx = clamp((e.clientX - (r.left + r.width / 2)) / (r.width * 2), -1, 1);
            const ny = clamp((e.clientY - (r.top + r.height / 2)) / (r.height * 2), -1, 1);
            try {
              av._av.lookAt(nx, ny);
            } catch {}
          }
        },
        onpointerleave: () => {
          if (av._av && av._av.lookAt) {
            try {
              av._av.lookAt(null);
            } catch {}
          }
        },
      };
      if (layout === 'tile') {
        parts.name = h('span', { class: 'tile-name' });
        el = h('div', Object.assign({ class: 'tile', role: 'listitem', tabindex: '-1' }, handlers), h('span', { class: 'tile-av' }, av, badge), parts.name);
      } else {
        parts.name = h('span', { class: 'row-name' });
        parts.preview = h('span', { class: 'row-preview' });
        parts.dot = h('span', { class: 'row-dot' });
        parts.kbd = h('span', { class: 'row-kbd' });
        parts.more = h(
          'button',
          {
            class: 'row-more',
            'aria-label': 'Bot actions',
            tabindex: '-1',
            onclick: (e) => {
              e.stopPropagation();
              const r = e.currentTarget.getBoundingClientRect();
              botMenu(S.bots.get(b.id), r.left, r.bottom + 4);
            },
            ondblclick: (e) => e.stopPropagation(),
          },
          icon('more', 16)
        );
        el = h(
          'div',
          Object.assign({ class: 'row-item', role: 'listitem', tabindex: '-1' }, handlers),
          h('span', { class: 'row-av' }, av, badge),
          h('span', { class: 'row-body' }, parts.name, parts.preview),
          parts.dot,
          parts.kbd,
          parts.more
        );
      }
      c = { el, parts, layout };
      rowCache.set(key, c);
    }
    updateRowEl(c, b);
    return c.el;
  }
  function updateRowEl(c, b) {
    const { el, parts, layout } = c;
    const active = b.id === S.current;
    const waiting = isWaiting(b);
    const unread = !active && b.unread > 0;
    el.classList.toggle('active', active);
    el.setAttribute('aria-current', active ? 'page' : 'false');
    el.setAttribute('aria-label', b.name + (b.main ? ', Main Bot' : '') + (b.running ? ', Working' : '') + (unread ? ', Unread activity' : ''));
    el.tabIndex = active ? 0 : -1;
    el.classList.toggle('has-dot', unread || waiting);
    parts.badge.classList.toggle('warn', waiting);
    parts.badge.hidden = !(unread || waiting);
    if (!parts.renaming && parts.name.textContent !== b.name) parts.name.textContent = b.name;
    el.title = layout === 'tile' || app.classList.contains('rail') ? b.name : '';
    syncAvatar(parts.av, b, layout === 'tile' ? 60 : 36, { live: true, state: botState(b), badge: b.main ? 'main' : null });
    if (layout === 'row') {
      const p = previewOf(b);
      const key = (p.warn ? 'w:' : '') + (p.draft != null ? 'd:' + p.draft : p.text);
      if (parts.previewKey !== key) {
        parts.previewKey = key;
        parts.preview.textContent = '';
        parts.preview.classList.toggle('warn', !!p.warn);
        if (p.draft != null) parts.preview.append(h('span', { class: 'draft-label' }, 'Draft:'), ' ' + p.draft);
        else parts.preview.textContent = p.text;
      }
      parts.dot.hidden = !(unread || waiting);
      parts.dot.classList.toggle('warn', waiting);
      parts.dot.setAttribute('aria-label', waiting ? 'Needs attention' : 'Unread activity');
    }
  }
  function reconcile(parent, want) {
    want.forEach((el, i) => {
      if (parent.children[i] !== el) parent.insertBefore(el, parent.children[i] || null);
    });
    while (parent.children.length > want.length) parent.lastElementChild.remove();
  }
  let tilesEl = null;
  let pinDivider = null;
  let hiddenHead = null;
  function renderSidebar() {
    const list = $('#botList');
    const all = sortedBots();
    const pins = pinnedIds();
    const vis = all.filter((b) => !b.hidden && !pins.includes(b.id));
    const hid = all.filter((b) => b.hidden);
    const want = [];
    const used = new Set();
    if (pins.length) {
      if (!tilesEl) tilesEl = h('div', { class: 'tiles', role: 'list', 'aria-label': 'Pinned' });
      const tiles = pins.map((id) => {
        used.add('tile:' + id);
        return botRowEl(S.bots.get(id), 'tile');
      });
      reconcile(tilesEl, tiles);
      // cap the tile row so few tiles stay centred (shell.md 3.3.1)
      tilesEl.style.maxWidth = pins.length < 3 ? pins.length * 99 - 4 + 'px' : '';
      want.push(tilesEl);
      if (!pinDivider) pinDivider = h('div', { class: 'pinned-divider' });
      want.push(pinDivider);
    }
    if (!all.length) want.push(h('div', { class: 'sb-empty' }, 'No saved Bots yet'));
    vis.forEach((b, i) => {
      used.add('row:' + b.id);
      const el = botRowEl(b, 'row');
      const c = rowCache.get('row:' + b.id);
      const n = pins.length + i + 1;
      const kbd = n <= 9 ? MOD + n : '';
      if (c.parts.kbd.textContent !== kbd) c.parts.kbd.textContent = kbd;
      want.push(el);
    });
    if (hid.length) {
      if (!hiddenHead) {
        hiddenHead = h('button', {
          class: 'sec-head',
          onclick: () => {
            S.hiddenOpen = !S.hiddenOpen;
            renderSidebar();
          },
        });
      }
      hiddenHead.textContent = '';
      hiddenHead.classList.toggle('open', S.hiddenOpen);
      hiddenHead.setAttribute('aria-expanded', S.hiddenOpen ? 'true' : 'false');
      hiddenHead.append(h('span', null, 'Hidden Bots'), h('span', { class: 'sec-count' }, h('span', { class: 'num' }, String(hid.length)), icon('chevronRight', 12, '', 2.2)));
      want.push(hiddenHead);
      if (S.hiddenOpen) {
        for (const b of hid) {
          used.add('row:' + b.id);
          const el = botRowEl(b, 'row');
          rowCache.get('row:' + b.id).parts.kbd.textContent = '';
          want.push(el);
        }
      }
    }
    reconcile(list, want);
    for (const [k] of rowCache) if (!used.has(k)) rowCache.delete(k);
    updateBadge();
    sweepAvatars();
  }
  function startRename(id, parts) {
    const b = S.bots.get(id);
    if (!b || parts.renaming) return;
    parts.renaming = true;
    const inp = h('input', { class: 'rename-input', value: b.name, maxlength: '60', 'aria-label': 'Rename Bot' });
    parts.name.textContent = '';
    parts.name.append(inp);
    inp.focus();
    inp.select();
    let done = false;
    const finish = async (save) => {
      if (done) return;
      done = true;
      parts.renaming = false;
      const v = inp.value.trim();
      parts.name.textContent = b.name;
      if (save && v && v !== b.name) await patchBot(b.id, { name: v });
      renderSidebar();
    };
    inp.addEventListener('keydown', (e) => {
      e.stopPropagation();
      if (e.key === 'Enter') finish(true);
      else if (e.key === 'Escape') finish(false);
    });
    inp.addEventListener('blur', () => finish(true));
    inp.addEventListener('click', (e) => e.stopPropagation());
    inp.addEventListener('dblclick', (e) => e.stopPropagation());
  }
  function copyText(text, msg = 'Copied') {
    const done = () => toast(msg);
    if (navigator.clipboard && navigator.clipboard.writeText) return navigator.clipboard.writeText(text).then(done, () => toast('Couldn’t copy', 'error'));
    try {
      const t = h('textarea', { style: 'position:fixed;opacity:0' });
      t.value = text;
      document.body.append(t);
      t.select();
      document.execCommand('copy');
      t.remove();
      done();
    } catch {
      toast('Couldn’t copy', 'error');
    }
  }
  // Sidebar "Bot actions" menu (settings-and-bots.md 4.9)
  function botMenu(b, x, y) {
    if (!b) return;
    const pinned = !!b.pinned && !b.hidden;
    if (isGroup(b)) {
      showMenu(
        [
          { label: pinned ? 'Unpin' : 'Pin', icon: pinned ? 'pinOff' : 'pin', action: () => togglePin(b) },
          b.unread > 0 ? { label: 'Mark as Read', icon: 'bell', action: () => markRead(b) } : b.id !== S.current && { label: 'Mark as Unread', icon: 'bell', action: () => markUnread(b) },
          '-',
          {
            label: 'Group settings',
            icon: 'pencil',
            action: () => {
              selectBot(b.id);
              setPanel(true, { focusName: true });
            },
          },
          '-',
          { label: 'Delete group chat', icon: 'trash', danger: true, action: () => deleteBot(b) },
        ],
        x,
        y
      );
      return;
    }
    const rename = () => {
      const c = rowCache.get((pinned ? 'tile:' : 'row:') + b.id);
      if (c && c.layout === 'row' && !app.classList.contains('rail')) startRename(b.id, c.parts);
      else {
        selectBot(b.id);
        setPanel(true, { focusName: true });
      }
    };
    showMenu(
      [
        !b.hidden && { label: pinned ? 'Unpin' : 'Pin', icon: pinned ? 'pinOff' : 'pin', action: () => togglePin(b) },
        b.unread > 0 ? { label: 'Mark as Read', icon: 'bell', action: () => markRead(b) } : b.id !== S.current && { label: 'Mark as Unread', icon: 'bell', action: () => markUnread(b) },
        '-',
        { label: 'Rename Bot', icon: 'pencil', action: rename },
        '-',
        b.conversation_id && { label: 'Copy conversation ID', icon: 'copy', action: () => copyText(b.conversation_id, 'Copied conversation ID') },
        b.conversation_id && '-',
        { label: b.hidden ? 'Show in sidebar' : 'Hide from sidebar', icon: b.hidden ? 'eye' : 'eyeOff', action: () => setHidden(b, !b.hidden) },
        b.main
          ? { label: 'Replace with different Bot', icon: 'refresh', action: () => choosePrimaryBot(b) }
          : { label: 'Delete', icon: 'trash', danger: true, action: () => deleteBot(b) },
      ],
      x,
      y
    );
  }
  // "Choose a primary Bot" (4.3): search + radio list
  function choosePrimaryBot(current) {
    let pick = current ? current.id : null;
    let q = '';
    const list = h('div', { class: 'radio-list', role: 'radiogroup', 'aria-label': 'Bots' });
    const search = h('input', { class: 'field', placeholder: 'Search Bots', 'aria-label': 'Search Bots', spellcheck: 'false' });
    const err = h('div', { class: 'dialog-error', role: 'alert', hidden: true });
    const okBtn = h('button', { class: 'btn primary', onclick: confirm }, 'Confirm');
    function paint() {
      list.textContent = '';
      const bots = sortedBots().filter((b) => !isGroup(b) && (!q || b.name.toLowerCase().includes(q)));
      if (!bots.length) list.append(h('div', { class: 'p-empty' }, `No matches for “${search.value.trim()}”`));
      for (const b of bots) {
        const on = pick === b.id;
        list.append(
          h(
            'button',
            {
              class: 'radio-item' + (on ? ' on' : ''),
              role: 'radio',
              'aria-checked': on ? 'true' : 'false',
              onclick: () => {
                pick = b.id;
                paint();
              },
            },
            avatar(b, 28),
            h('span', { class: 'radio-item-name' }, b.name),
            h('span', { class: 'radio-dot' })
          )
        );
      }
      okBtn.disabled = !pick || (current && pick === current.id);
    }
    search.addEventListener('input', () => {
      q = search.value.trim().toLowerCase();
      paint();
    });
    async function confirm() {
      if (!pick) return;
      okBtn.disabled = true;
      err.hidden = true;
      try {
        await api(`/api/bots/${pick}/make-main`, { method: 'POST' });
        for (const b of S.bots.values()) b.main = b.id === pick;
        renderSidebar();
        close();
      } catch (e) {
        err.textContent = 'Couldn’t set the primary Bot';
        err.hidden = false;
        okBtn.disabled = false;
      }
    }
    const box = h(
      'div',
      { class: 'dialog', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Choose a primary Bot' },
      h('button', { class: 'icon-btn dialog-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 16)),
      h('h2', { class: 'dialog-title' }, 'Choose a primary Bot'),
      h('p', { class: 'dialog-body' }, 'Your main point of contact. It checks in proactively and coordinates work with your other Bots.'),
      h('div', { style: 'margin-top:14px' }, search),
      list,
      err,
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn', onclick: () => close() }, 'Cancel'), okBtn)
    );
    const close = openModal(box);
    paint();
    setTimeout(() => search.focus(), 40);
  }
  async function togglePin(b) {
    const pin = !b.pinned;
    const order = (Array.isArray(S.prefs.pinOrder) ? S.prefs.pinOrder : []).filter((id) => id !== b.id && S.bots.has(id));
    if (pin) order.push(b.id);
    S.prefs.pinOrder = order;
    savePrefs();
    b.pinned = pin;
    renderSidebar();
    const nb = await patchBot(b.id, { pinned: pin });
    if (!nb) {
      b.pinned = !pin;
      renderSidebar();
    }
  }
  // one-time move of pins kept in prefs (older builds) onto the bots themselves
  function migratePins() {
    const old = Array.isArray(S.prefs.pinned) ? S.prefs.pinned : null;
    if (!old) return;
    delete S.prefs.pinned;
    S.prefs.pinOrder = old.filter((id) => S.bots.has(id));
    savePrefs();
    for (const id of S.prefs.pinOrder) {
      const b = S.bots.get(id);
      if (b && !b.pinned) {
        b.pinned = true;
        api(`/api/bots/${id}`, { method: 'PATCH', body: { pinned: true } }).catch(() => {});
      }
    }
  }
  async function markRead(b) {
    b.unread = 0;
    renderSidebar();
    api(`/api/bots/${b.id}/read`, { method: 'POST' }).catch(() => {});
  }
  async function markUnread(b) {
    b.unread = 1;
    renderSidebar();
    api(`/api/bots/${b.id}/unread`, { method: 'POST' }).catch(() => {});
  }
  async function patchBot(id, body) {
    try {
      const nb = await api(`/api/bots/${id}`, { method: 'PATCH', body });
      if (nb && nb.id) {
        const prev = S.bots.get(nb.id);
        S.bots.set(nb.id, nb);
        trackRun(nb, prev);
        renderSidebar();
        if (nb.id === S.current) {
          renderChatHeader();
          applyTint();
          updateComposer();
          scheduleRender();
          if (S.panelOpen) refreshPanelBot();
        }
      }
      return nb;
    } catch (e) {
      fail(e);
      return null;
    }
  }
  async function setHidden(b, hidden) {
    const nb = await patchBot(b.id, { hidden });
    if (nb) toast(hidden ? `${b.name} is hidden` : `${b.name} is back in your sidebar`);
  }
  async function resetBot(b) {
    const ok = await confirmDialog({
      title: 'Reset this conversation?',
      body: 'The conversation history starts over. The Bot\'s memory, files, routines, and settings are kept.',
      confirmText: 'Reset',
      danger: true,
    });
    if (!ok) return;
    try {
      await api(`/api/bots/${b.id}/reset`, { method: 'POST' });
      S.msgs.set(b.id, []);
      S.hasMore.set(b.id, false);
      if (S.current === b.id) renderMessages(true);
    } catch (e) {
      fail(e);
    }
  }
  async function deleteBot(b) {
    if (b.main) return choosePrimaryBot(b);
    const ok = await confirmDialog({
      title: `Delete “${b.name}”`,
      body: isGroup(b)
        ? 'This deletes the group chat and its messages. The Bots and their own chats stay. This can\'t be undone.'
        : 'This permanently deletes the Bot and its chat history. This can\'t be undone.',
      confirmText: 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await api(`/api/bots/${b.id}`, { method: 'DELETE' });
      onEvent({ type: 'bot.deleted', bot_id: b.id });
    } catch (e) {
      toast(`Couldn’t delete “${b.name}”`, 'error');
    }
  }
  // sidebar list fade: no top fade while scrolled to the top
  $('#sbScroll').addEventListener('scroll', (e) => e.currentTarget.classList.toggle('at-top', e.currentTarget.scrollTop < 2));
  $('#sbScroll').classList.add('at-top');

  // ───────────────────────────── chat: selection, loading ─────────────────────────────
  function loadDraftStore() {
    try {
      const raw = JSON.parse(localStorage.getItem('openbot.drafts') || '{}');
      for (const k in raw) if (raw[k]) S.drafts.set(k, { text: raw[k], pending: [] });
    } catch {}
  }
  const persistDrafts = debounce(() => {
    const o = {};
    for (const [k, v] of S.drafts) if (v.text) o[k] = v.text;
    try {
      localStorage.setItem('openbot.drafts', JSON.stringify(o));
    } catch {}
  }, 500);
  function saveDraft() {
    const b = curBot();
    if (!b) return;
    if (input.value || S.pending.length) S.drafts.set(b.id, { text: input.value, pending: S.pending });
    else S.drafts.delete(b.id);
    persistDrafts();
  }
  function restoreDraft() {
    const d = S.current && S.drafts.get(S.current);
    input.value = d ? d.text : '';
    S.pending = d ? d.pending || [] : [];
    S.composerExpanded = false;
    renderAttachRow();
    renderReplyRow();
    updateComposer();
  }
  async function selectBot(id, { focus = true } = {}) {
    if (!S.bots.has(id)) return;
    if (S.current !== id) {
      saveDraft();
      S.current = id;
      S.prefs.lastBotId = id;
      savePrefs();
      restoreDraft();
      S.nodes.clear();
      S.seps.clear();
      S.newCount = 0;
      S.newDiv = null;
      S.initialRender = true;
      const b = S.bots.get(id);
      if (b.unread > 0) S.newDiv = { botId: id, unread: b.unread, msgId: null };
    }
    renderSidebar();
    applyTint();
    renderChat();
    if (S.panelOpen) renderPanel();
    app.classList.remove('sidebar-open');
    if (focus && !isPhone()) input.focus({ preventScroll: true });
    if (!S.msgs.has(id)) {
      await loadMessages(id);
      if (S.current === id) renderMessages(true);
    }
    markReadSoon();
  }
  async function loadMessages(id, before) {
    if (S.loading.has(id)) return;
    S.loading.add(id);
    try {
      const limit = 200;
      const list = (await api(`/api/bots/${id}/messages?limit=${limit}${before ? '&before=' + before : ''}`)) || [];
      const cur = S.msgs.get(id);
      if (before && cur) {
        const ids = new Set(cur.map((m) => m.id));
        S.msgs.set(id, [...list.filter((m) => !ids.has(m.id)), ...cur]);
      } else if (cur && cur.length) {
        const ids = new Set(list.map((m) => m.id));
        S.msgs.set(id, [...list, ...cur.filter((m) => !ids.has(m.id) && !m._tmp)]);
      } else S.msgs.set(id, list);
      S.hasMore.set(id, list.length >= limit);
    } catch (e) {
      if (!S.msgs.has(id)) S.msgs.set(id, []);
      fail(e);
    } finally {
      S.loading.delete(id);
    }
  }

  function renderChat() {
    renderChatHeader();
    renderMessages(true);
    updateComposer();
  }
  let islandAv = null;
  function renderChatHeader() {
    const pill = $('#island');
    const b = curBot();
    if (!b) {
      pill.hidden = true;
      return;
    }
    pill.hidden = false;
    if (!islandAv) {
      islandAv = h('span', { class: 'av', style: { width: '24px', height: '24px' } });
      islandAv._size = 24;
      pill.append(islandAv, h('span', { class: 'island-name' }), h('span', { class: 'island-chev' }, icon('chevronRight', 12, '', 2.6)));
    }
    syncAvatar(islandAv, b, 24);
    const nm = $('.island-name', pill);
    if (nm.textContent !== b.name) nm.textContent = b.name;
    pill.title = b.name;
    pill.setAttribute('aria-expanded', S.panelOpen ? 'true' : 'false');
    pill.dataset.tip = isGroup(b) ? 'Group settings' : 'Bot settings';
    pill.dataset.kbd = isMac ? '⌘⇧,' : 'Ctrl+Shift+,';
    renderScreenBtn();
  }

  // ───────────────────────────── chat: transcript ─────────────────────────────
  const msgsEl = $('#messages');
  const colEl = $('#msgInner');
  const distFromBottom = () => msgsEl.scrollHeight - msgsEl.scrollTop - msgsEl.clientHeight;
  function scrollBottom(smooth) {
    msgsEl.scrollTo({ top: msgsEl.scrollHeight, behavior: smooth ? 'smooth' : 'auto' });
    S.pinned = true;
    S.newCount = 0;
    updateJump();
  }
  function updateJump() {
    const d = distFromBottom();
    const pill = $('#newPill');
    const btn = $('#toBottom');
    if (d < 8) {
      S.newCount = 0;
    }
    pill.hidden = !(S.newCount > 0);
    if (!pill.hidden) {
      pill.textContent = '';
      pill.append(icon('arrowDown', 20), h('span', null, S.newCount === 1 ? 'New message' : `${S.newCount} new messages`));
      pill.append(
        h(
          'button',
          {
            class: 'x',
            'aria-label': 'Dismiss new messages',
            onclick: (e) => {
              e.stopPropagation();
              S.newCount = 0;
              updateJump();
            },
          },
          icon('x', 12, '', 2.4)
        )
      );
    }
    btn.hidden = !pill.hidden || d <= 80 || !curBot();
  }
  // keep the view pinned to the bottom when the column changes size (panel, sidebar, window)
  new ResizeObserver(() => {
    if (S.pinned) msgsEl.scrollTop = msgsEl.scrollHeight;
  }).observe(msgsEl);
  let lastBox = '';
  msgsEl.addEventListener('scroll', () => {
    // A message menu belongs next to its message: close it when the chat scrolls away.
    if (openMenuEl && openMenuEl.querySelector('.react-box')) closeMenu();
    const box = msgsEl.clientWidth + 'x' + msgsEl.clientHeight;
    if (box === lastBox) S.pinned = distFromBottom() < 8;
    else if (S.pinned) msgsEl.scrollTop = msgsEl.scrollHeight;
    lastBox = box;
    updateJump();
    maybeLoadOlder();
  });
  const maybeLoadOlder = debounce(async () => {
    const b = curBot();
    if (!b || msgsEl.scrollTop > 120 || !S.hasMore.get(b.id) || S.loading.has(b.id)) return;
    const list = S.msgs.get(b.id) || [];
    const oldest = list.find((m) => !m._tmp);
    if (!oldest) return;
    const prevH = msgsEl.scrollHeight;
    const prevTop = msgsEl.scrollTop;
    renderMessages(false);
    await loadMessages(b.id, oldest.created_at);
    if (S.current !== b.id) return;
    renderMessages(false);
    msgsEl.scrollTop = msgsEl.scrollHeight - prevH + prevTop;
  }, 120);

  const isShown = (m) => {
    if (m.role === 'tool' || m.role === 'note') return S.prefs.showTools === true;
    return true;
  };
  const speakerOf = (role) => (role === 'user' ? 'user' : role === 'event' || role === 'system' || role === 'relay' ? 'center' : 'bot');
  // a widget is "open" when it has no answer yet
  const openWidget = (list) => {
    for (let i = list.length - 1; i >= 0; i--) {
      const m = list[i];
      if (m.role === 'widget') return m.meta && m.meta.answer == null && m.status !== 'answered' ? m : null;
      if (m.role === 'form') return m.status === 'pending' ? m : null;
      if (m.role === 'user') return null;
    }
    return null;
  };
  function sigOf(m) {
    const meta = m.meta || {};
    return [m.role, m.status, m.text ? m.text.length : 0, m.html ? m.html.length : 0, meta.answer == null ? '' : meta.answer, m._tmp ? 1 : 0, (m.role === 'tool' || m.role === 'relay') && S.expanded.has(m.id) ? 1 : 0, m.role === 'approval' ? approvalAnswers.get(meta.approval_id) || '' : '', reactionsOf(m).map((r) => r.emoji + r.by).join(',')].join('|');
  }

  let renderQueued = false;
  function scheduleRender() {
    if (renderQueued) return;
    renderQueued = true;
    requestAnimationFrame(() => {
      renderQueued = false;
      renderMessages(false);
    });
  }

  let emptyEl = null;
  function renderMessages(toBottom) {
    const b = curBot();
    const wasPinned = S.pinned || distFromBottom() < 8;
    if (!b) {
      colEl.textContent = '';
      S.nodes.clear();
      colEl.append(noBotState());
      updateJump();
      return;
    }
    const list = S.msgs.get(b.id);
    if (!list) {
      reconcile(colEl, [h('div', { class: 'tx-spacer' }), h('div', { class: 'load-older' }, spinner('lg'))]);
      return;
    }
    const shown = list.filter(isShown);
    const want = [h('div', { class: 'tx-spacer' })];
    want[0] = colEl.firstElementChild && colEl.firstElementChild.classList.contains('tx-spacer') ? colEl.firstElementChild : want[0];
    const act = activityFor(b, list);
    if (!shown.length && !act) {
      if (!emptyEl || emptyEl._bot !== b.id) emptyEl = emptyChat(b);
      reconcile(colEl, [emptyEl]);
      S.nodes.clear();
      updateJump();
      sweepAvatars();
      return;
    }
    if (S.hasMore.get(b.id)) {
      want.push(h('div', { class: 'load-older' }, S.loading.has(b.id) ? spinner('lg') : ''));
    }
    // NEW divider: before the first unread message (counted from the end)
    if (S.newDiv && S.newDiv.botId === b.id && !S.newDiv.msgId && S.newDiv.unread > 0) {
      let n = 0;
      for (let i = shown.length - 1; i >= 0; i--) {
        if (shown[i].role === 'user') break;
        n++;
        if (n >= S.newDiv.unread) {
          S.newDiv.msgId = shown[i].id;
          break;
        }
      }
      if (!S.newDiv.msgId && n > 0) S.newDiv.msgId = shown[shown.length - n].id;
    }
    const seen = new Set();
    let prev = null;
    for (const m of shown) {
      if (!prev || m.created_at - prev.created_at >= 15 * 60 * 1000 || !sameDay(prev.created_at, m.created_at)) {
        const key = 't:' + m.id;
        let sep = S.seps.get(key);
        if (!sep) {
          sep = h('div', { class: 'tsep', role: 'separator' }, timeLabel(m.created_at));
          S.seps.set(key, sep);
        }
        want.push(sep);
      }
      if (S.newDiv && S.newDiv.botId === b.id && S.newDiv.msgId === m.id) {
        let nd = S.seps.get('new');
        if (!nd) {
          nd = h('div', { class: 'newdiv', role: 'separator', 'aria-label': 'New messages' }, h('span', null, 'New'));
          S.seps.set('new', nd);
        }
        want.push(nd);
      }
      want.push(rowFor(m));
      seen.add(m.id);
      prev = m;
    }
    if (act) want.push(act);
    reconcile(colEl, want);
    for (const id of [...S.nodes.keys()]) if (!seen.has(id)) S.nodes.delete(id);
    regroup();
    S.initialRender = false;
    if (toBottom || wasPinned) {
      msgsEl.scrollTop = msgsEl.scrollHeight;
      S.pinned = true;
      if (toBottom) requestAnimationFrame(() => (msgsEl.scrollTop = msgsEl.scrollHeight));
    }
    updateJump();
    sweepAvatars();
  }
  function rowFor(m) {
    const sig = sigOf(m);
    const c = S.nodes.get(m.id);
    if (c && c.sig === sig) return c.el;
    if (c && m.role === 'bot' && c.role === 'bot') {
      const bubble = $('.bubble', c.el);
      if (bubble) {
        fillBotBubble(bubble, m);
        const w = bubble.closest('.bwrap');
        if (w) paintPills(w, m);
        c.el.classList.toggle('reacted', reactionsOf(m).length > 0);
        c.sig = sig;
        return c.el;
      }
    }
    const el = msgNode(m);
    if (!c && !S.initialRender) el.classList.add('enter');
    if (c) el.className = el.className + (c.el.classList.contains('gs') ? ' gs' : '');
    S.nodes.set(m.id, { el, sig, role: m.role });
    return el;
  }
  // group spacing (12px at a speaker change, 4px inside a run) and touching corners (6px)
  function regroup() {
    const rows = Array.from(colEl.children);
    for (let i = 0; i < rows.length; i++) {
      const n = rows[i];
      const sp = n.dataset.speaker;
      if (!sp) continue;
      const p = rows[i - 1];
      const nx = rows[i + 1];
      const pSp = p && p.dataset.speaker;
      const nSp = nx && nx.dataset.speaker;
      n.classList.toggle('gs', !!(pSp && pSp !== sp && sp !== 'center'));
      if (n.classList.contains('grp')) {
        n.classList.toggle('run-first', pSp !== sp);
        n.classList.toggle('run-last', nSp !== sp);
      }
      const bubble = n.querySelector(':scope > .stack > .bwrap:last-child > .bubble, :scope > .gcol > .stack > .bwrap:last-child > .bubble');
      const firstBubble = n.querySelector(':scope > .stack > .bwrap:first-child > .bubble, :scope > .gcol > .stack > .bwrap:first-child > .bubble');
      if (firstBubble) firstBubble.classList.toggle('cp', !!(pSp === sp && sp !== 'center' && p.dataset.bubbleEnd === '1'));
      if (bubble) bubble.classList.toggle('cn', !!(nSp === sp && sp !== 'center' && nx.dataset.bubbleStart === '1'));
    }
  }

  // working indicator: animated 28px mark + shimmering activity phrase + elapsed time (shell.md 4.7)
  let actEl = null;
  function activityFor(b, list) {
    if (!b.running) return null;
    const last = list[list.length - 1];
    if (last && last.role === 'approval' && last.status === 'pending') return null;
    if (last && (last.role === 'widget' || last.role === 'form') && openWidget(list)) return null;
    if (!actEl) {
      const av = h('span', { class: 'av', style: { width: '28px', height: '28px' } });
      av._size = 28;
      av.addEventListener('click', () => pokeAvatar(av));
      const phrase = h('span', { class: 'act-phrase shimmer' });
      const elapsed = h('span', { class: 'act-elapsed' });
      actEl = h(
        'div',
        { class: 'row bot', 'data-speaker': 'bot', 'data-bubble-start': '1', role: 'status' },
        h('div', { class: 'activity' }, av, h('span', { class: 'act-text' }, phrase, elapsed))
      );
      const watch = h('button', { class: 'act-watch', hidden: true, 'data-tip': 'Watch its screen', onclick: () => openScreen(actEl._bot) }, icon('monitor', 13, '', 2), 'Watch');
      $('.activity', actEl).append(watch);
      actEl._parts = { av, phrase, elapsed, watch };
    }
    const { av, phrase, watch } = actEl._parts;
    actEl._bot = b.id;
    watch.hidden = !(b.activity && b.activity.kind === 'on_its_computer');
    if (av._bot !== b.id) {
      av._bot = b.id;
      av._look = null;
      phrase.textContent = '';
      phrase._at = 0;
    }
    // In a group: the pictures of the Bots still working.
    const working = isGroup(b) && b.activity && Array.isArray(b.activity.members) ? Object.assign({}, b, { members: b.activity.members }) : b;
    syncAvatar(av, working, 28, { live: true, state: botState(b) });
    const start = S.runStart.get(b.id) || Date.now();
    const text = (b.activity && b.activity.label) || WORK_PHRASES[Math.floor(start / 1000) % WORK_PHRASES.length];
    setPhrase(phrase, text);
    tickActivity();
    return actEl;
  }
  function setPhrase(phrase, text) {
    phrase._want = text;
    if (phrase.textContent === text) return;
    const wait = (phrase._at || 0) + HOLD_MS - Date.now();
    if (phrase.textContent && wait > 0) {
      if (!phrase._t) {
        phrase._t = setTimeout(() => {
          phrase._t = null;
          setPhrase(phrase, phrase._want);
        }, wait);
      }
      return;
    }
    const had = !!phrase.textContent;
    phrase.textContent = text;
    phrase._at = Date.now();
    if (had) {
      phrase.classList.remove('swap');
      void phrase.offsetWidth;
      phrase.classList.add('swap');
    }
  }
  function tickActivity() {
    if (!actEl || !actEl.isConnected) return;
    const b = curBot();
    if (!b) return;
    const start = S.runStart.get(b.id);
    const ms = start ? Date.now() - start : 0;
    actEl._parts.elapsed.textContent = ms >= 5000 ? ` · ${fmtElapsed(ms)}` : '';
  }
  setInterval(tickActivity, 1000);

  function noBotState() {
    return h(
      'div',
      { class: 'empty-chat' },
      avatar({ id: 'new', shape: 'ball', color: 'violet' }, 72, { live: true }),
      h('div', { class: 'empty-name' }, 'Create your first Bot'),
      h('div', { class: 'empty-sub' }, 'Bots are Claude-powered helpers that work on your computer. Each one gets its own folder and remembers your chat.'),
      h('div', { class: 'empty-actions' }, h('button', { class: 'btn primary', onclick: () => openNewBot() }, icon('plus', 15, '', 2), 'New Bot'), h('button', { class: 'btn', onclick: () => openTemplates() }, 'Browse templates'))
    );
  }
  function emptyChat(b) {
    const av = avatar(b, 72, { live: true, interactive: true });
    av.addEventListener('click', () => pokeAvatar(av));
    const sub = isGroup(b) ? memberNames(b) : 'No messages yet';
    const el = h('div', { class: 'empty-chat', role: 'note' }, av, h('div', { class: 'empty-name' }, b.name), h('div', { class: 'empty-sub' }, sub));
    el._bot = b.id;
    return el;
  }

  // ── message rows
  const EMOJI_ONLY = /^(?:\p{Extended_Pictographic}(?:️|\p{Emoji_Modifier}|‍\p{Extended_Pictographic})*\s*){1,3}$/u;
  function msgNode(m) {
    const sp = speakerOf(m.role);
    const wrap = h('div', { class: 'row ' + sp, 'data-speaker': sp, 'data-id': m.id, 'data-role': m.role });
    const stack = h('div', { class: 'stack' });
    wrap.append(stack);
    let bubbleStart = false;
    let bubbleEnd = false;
    const title = m.created_at ? fmtTime(m.created_at) : null;
    switch (m.role) {
      case 'user': {
        const rt = m.meta && m.meta.reply_to;
        if (rt && rt.id) stack.append(h('button', { class: 'quote-pill', 'data-tip': 'Jump to replied message', onclick: () => jumpTo(rt.id) }, icon('reply', 12, '', 2), h('span', { class: 'qp-who' }, rt.who), h('span', { class: 'qp-text' }, rt.quote)));
        const atts = (m.meta && m.meta.attachments) || [];
        const imgs = atts.filter(isImage);
        const files = atts.filter((a) => !isImage(a));
        if (imgs.length) stack.append(imageRow(imgs));
        for (const f of files) stack.append(fileCard(f));
        if (m.text) {
          const t = m.text;
          const emoji = EMOJI_ONLY.test(t.trim());
          const bubble = h('div', { class: 'bubble user' + (m._tmp ? ' sending' : '') + (emoji ? ' emoji' : !emoji && [...t.trim()].length <= 2 ? ' glyph' : ''), title }, t);
          stack.append(bwrap(bubble, t, m));
          bubbleEnd = true;
          bubbleStart = !imgs.length && !files.length;
        }
        const qrow = queuedRow(m);
        if (qrow) stack.append(qrow);
        break;
      }
      case 'bot': {
        const emoji = !m.html && EMOJI_ONLY.test((m.text || '').trim());
        const bubble = h('div', { class: 'bubble agent' + (emoji ? ' emoji' : ''), title });
        fillBotBubble(bubble, m);
        stack.append(bwrap(bubble, null, m));
        bubbleStart = bubbleEnd = true;
        break;
      }
      case 'attachment':
        stack.append(...attachmentMsg(m));
        if (m.text) {
          stack.append(bwrap(h('div', { class: 'bubble agent', title }, m.text), m.text, m));
          bubbleEnd = true;
        }
        break;
      case 'widget':
        stack.append(widgetCard(m));
        break;
      case 'form':
        stack.append(formCard(m));
        break;
      case 'visualization':
        stack.append(vizCard(m));
        break;
      case 'approval':
        stack.append(approvalCard(m));
        break;
      case 'calendar':
        stack.append(calendarCard(m));
        break;
      case 'tool':
        stack.append(toolView(m));
        break;
      case 'error':
        stack.append(h('div', { class: 'bwrap' }, h('div', { class: 'bubble err', role: 'alert' }, icon('alert', 15), h('span', null, m.text || 'Something went wrong.'))));
        if (m.meta && Array.isArray(m.meta.actions) && m.meta.actions.length && isLastError(m)) stack.append(errorActions(m));
        break;
      case 'event':
        stack.append(eventLine(m));
        break;
      case 'relay':
        stack.append(relayRow(m));
        break;
      case 'note':
        stack.append(h('div', { class: 'note-line', title }, m.text || ''));
        break;
      default:
        stack.append(h('div', { class: 'event-line', title }, m.text));
    }
    if (bubbleStart) wrap.dataset.bubbleStart = '1';
    if (bubbleEnd) wrap.dataset.bubbleEnd = '1';
    wrap.classList.toggle('reacted', reactionsOf(m).length > 0);
    // In a group chat each Bot's messages carry its name (above a run) and picture (beside its end).
    const speaker = sp === 'bot' && m.meta && m.meta.speaker;
    if (speaker) {
      const who = S.bots.get(speaker) || { id: speaker, name: 'A Bot' };
      wrap.dataset.speaker = 'bot:' + speaker;
      wrap.classList.add('grp');
      wrap.textContent = '';
      wrap.append(
        h('span', { class: 'gav', 'aria-hidden': 'true' }, avatar(who, 22)),
        h('div', { class: 'gcol' }, h('div', { class: 'sender', style: { color: inkOf(lookOf(who).color) } }, who.name), stack)
      );
    }
    return wrap;
  }
  function bwrap(bubble, copyText, m) {
    const w = h('div', { class: 'bwrap' }, bubble);
    const copy = () => {
      const txt = copyText != null ? copyText : (m && m.text) || bubble.innerText;
      navigator.clipboard.writeText(txt).then(() => toast('Copied'), () => toast('Couldn’t copy', 'error'));
    };
    // A saved message gets reactions and Reply; one still sending only Copy.
    const real = m && !m._tmp && m.id && !String(m.id).startsWith('tmp-');
    const bar = h('div', { class: 'hover-bar', role: 'toolbar', 'aria-label': 'Message actions' });
    if (real) {
      bar.append(
        h('button', { class: 'hb-btn', 'aria-label': 'Add reaction', 'data-tip': 'Add reaction', onclick: (e) => menuAt(e.currentTarget, [{ el: reactBar(m) }]) }, icon('smile', 15)),
        h('button', { class: 'hb-btn', 'aria-label': 'More message actions', 'data-tip': 'More', onclick: (e) => menuAt(e.currentTarget, messageMenu(m, copy)) }, icon('more', 15))
      );
      bubble.addEventListener('contextmenu', (e) => {
        if (window.getSelection && String(window.getSelection()).trim()) return;
        e.preventDefault();
        showMenu(messageMenu(m, copy), e.clientX, e.clientY);
      });
      paintPills(w, m);
    } else bar.append(h('button', { class: 'hb-btn', 'aria-label': 'Copy', 'data-tip': 'Copy', onclick: copy }, icon('copy', 15)));
    w.append(bar);
    return w;
  }

  // ── reactions and replies (shell.md 4.6, 6.12)
  const QUICK_EMOJI = ['👍', '👎', '❤️', '😂', '🎉'];
  const MORE_EMOJI = ['😊', '😮', '😢', '🙏', '🔥', '👏', '💯', '✅', '👀', '🤔', '😅', '🥳', '😍', '🙌', '💪', '👌', '🤝', '⭐', '😎', '🤯', '😭', '😬', '🫡', '💡', '🚀', '☕', '🍕', '📌', '❓', '❗'];
  const reactionsOf = (m) => (m && m.meta && Array.isArray(m.meta.reactions) ? m.meta.reactions : []);
  const mine = (m, e) => reactionsOf(m).some((r) => r.by === 'user' && r.emoji === e);
  async function react(m, emoji) {
    const cur = (S.msgs.get(m.bot_id) || []).find((x) => x.id === m.id) || m;
    try {
      const nm = await api(`/api/messages/${m.id}/react`, { method: 'POST', body: { emoji, on: !mine(cur, emoji) } });
      if (nm && nm.id) upsertMessage(nm, false);
    } catch (e) {
      fail(e);
    }
  }
  // Five quick emojis and a + for more.
  function reactBar(m) {
    const cur = () => (S.msgs.get(m.bot_id) || []).find((x) => x.id === m.id) || m;
    const pick = (e) => {
      closeMenu();
      react(m, e);
    };
    const cell = (e) => h('button', { class: 'react-q' + (mine(cur(), e) ? ' on' : ''), 'aria-label': 'React ' + e, onclick: () => pick(e) }, e);
    const bar = h('div', { class: 'react-bar' }, ...QUICK_EMOJI.map(cell));
    const grid = h('div', { class: 'react-grid', hidden: true }, ...MORE_EMOJI.map(cell));
    bar.append(
      h('button', {
        class: 'react-q more',
        'aria-label': 'More emoji',
        onclick: (e) => {
          grid.hidden = !grid.hidden;
          e.currentTarget.classList.toggle('on', !grid.hidden);
          // Keep the bigger menu inside the window.
          const menu = e.currentTarget.closest('.menu');
          const r = menu && menu.getBoundingClientRect();
          if (r && r.bottom > innerHeight - 8) menu.style.top = Math.max(8, innerHeight - r.height - 8) + 'px';
        },
      }, icon('plus', 14, '', 2))
    );
    return h('div', { class: 'react-box' }, bar, grid);
  }
  function messageMenu(m, copy) {
    return [{ el: reactBar(m) }, '-', { label: 'Reply', icon: 'reply', action: () => startReply(m) }, { label: 'Copy', icon: 'copy', action: copy }];
  }
  // Tapback pills on the bubble's top corner: one per emoji, at most 3, yours kept in view.
  function paintPills(w, m) {
    let box = w.querySelector(':scope > .react-pills');
    const list = reactionsOf(m);
    if (!list.length) {
      if (box) box.remove();
      return;
    }
    if (!box) {
      box = h('div', { class: 'react-pills' });
      w.append(box);
    }
    box.textContent = '';
    const groups = new Map();
    for (const r of list) {
      const g = groups.get(r.emoji) || { emoji: r.emoji, by: [] };
      g.by.push(r.by);
      groups.set(r.emoji, g);
    }
    const all = [...groups.values()].sort((a, b) => b.by.includes('user') - a.by.includes('user') || b.by.length - a.by.length);
    for (const g of all.slice(0, 3)) {
      const names = g.by.map((b) => (b === 'user' ? 'You' : (S.bots.get(b) || {}).name || 'A Bot')).join(', ');
      box.append(
        h(
          'button',
          { class: 'react-pill' + (g.by.includes('user') ? ' on' : ''), 'data-tip': names, 'aria-label': `${g.emoji} from ${names}`, onclick: () => react(m, g.emoji) },
          h('span', { class: 'rp-emoji' }, g.emoji),
          g.by.length > 1 ? h('span', { class: 'rp-count' }, String(g.by.length)) : null
        )
      );
    }
  }
  // Reply: a pill above the message box; the sent message quotes the original.
  function replyInfo(m) {
    const who = m.role === 'user' ? 'You' : (ownerOf(m) || {}).name || 'Bot';
    // Plain text, like the sidebar preview (no Markdown marks).
    const quote = (m.text || (m.meta && m.meta.name) || 'a message').replace(/\s+/g, ' ').replace(/[*`#>]/g, '').replace(/^[-+\s]+/, '').trim();
    return { id: m.id, who, quote: quote.length > 140 ? quote.slice(0, 139) + '…' : quote };
  }
  function startReply(m) {
    S.reply = Object.assign({ botId: m.bot_id }, replyInfo(m));
    renderReplyRow();
    updateComposer();
    input.focus();
  }
  function cancelReply() {
    S.reply = null;
    renderReplyRow();
    updateComposer();
  }
  function jumpTo(id) {
    const c = S.nodes.get(id);
    if (!c || !c.el.isConnected) return toast('That message isn’t loaded here');
    c.el.scrollIntoView({ block: 'center', behavior: 'smooth' });
    c.el.classList.remove('flash');
    void c.el.offsetWidth;
    c.el.classList.add('flash');
  }
  function renderReplyRow() {
    let row = $('#replyRow');
    if (!row) {
      row = h('div', { class: 'c-reply', id: 'replyRow', hidden: true });
      $('#cContent').insertBefore(row, $('#attachRow'));
    }
    const r = S.reply && S.reply.botId === S.current ? S.reply : null;
    row.hidden = !r;
    row.textContent = '';
    if (!r) return;
    row.append(
      h('button', { class: 'c-reply-body', type: 'button', 'data-tip': 'Jump to message', onclick: () => jumpTo(r.id) }, icon('reply', 13, '', 2), h('span', { class: 'c-reply-who' }, `Reply to ${r.who}`), h('span', { class: 'c-reply-quote' }, r.quote)),
      h('button', { class: 'pend-x', type: 'button', 'aria-label': 'Cancel reply', onclick: () => cancelReply() }, icon('x', 12, '', 2.4))
    );
  }
  function fillBotBubble(bubble, m) {
    if (bubble.classList.contains('emoji')) {
      bubble.textContent = m.text;
      return;
    }
    const md = h('div', { class: 'md' });
    if (m.html) md.innerHTML = m.html;
    else md.append(h('p', null, m.text || ''));
    for (const a of md.querySelectorAll('a[href]')) {
      a.target = '_blank';
      a.rel = 'noopener noreferrer';
    }
    for (const pre of md.querySelectorAll('pre')) {
      const fig = h('div', { class: 'code-fig' });
      pre.replaceWith(fig);
      const copyBtn = h(
        'button',
        {
          class: 'code-copy',
          'aria-label': 'Copy code',
          'data-tip': 'Copy code',
          onclick: () => navigator.clipboard.writeText(pre.innerText).then(() => toast('Copied'), () => toast('Couldn’t copy', 'error')),
        },
        icon('copy', 14)
      );
      fig.append(pre, copyBtn);
    }
    mentionify(md);
    bubble.textContent = '';
    bubble.append(md);
  }
  function isImage(a) {
    return (a.mime && a.mime.startsWith('image/')) || /\.(png|jpe?g|gif|webp|bmp|svg|heic|avif)$/i.test(a.name || a.path || '');
  }
  // up to 3 tiles per row; extras collapse into "+N" on the last tile (shell.md 4.4.3)
  function imageRow(imgs) {
    const urls = imgs.map((a) => ({ url: a.localUrl || fileUrl(a.url), name: a.name || 'image' }));
    const showN = Math.min(3, urls.length);
    const row = h('div', { class: 'img-row' });
    for (let i = 0; i < showN; i++) {
      const u = urls[i];
      const extra = i === showN - 1 ? urls.length - showN : 0;
      const img = h('img', {
        src: u.url,
        alt: u.name,
        loading: 'lazy',
        onload: () => {
          tile.classList.remove('loading');
          if (S.pinned) msgsEl.scrollTop = msgsEl.scrollHeight;
        },
        onerror: () => tile.classList.remove('loading'),
      });
      const tile = h(
        'button',
        { class: 'img-tile loading', 'aria-label': extra ? `Open the remaining ${extra} images full screen` : u.name, onclick: () => openLightbox(urls, extra ? showN : i) },
        img,
        extra ? h('span', { class: 'img-more' }, '+' + extra) : null
      );
      row.append(tile);
    }
    return row;
  }
  function extOf(name) {
    const m = /\.([a-z0-9]{1,5})$/i.exec(name || '');
    return m ? m[1].toUpperCase() : 'FILE';
  }
  function fileCard(a) {
    const url = a.localUrl || fileUrl(a.url);
    const meta = [extOf(a.name || a.path), a.size ? fmtBytes(a.size) : ''].filter(Boolean).join(' · ');
    return h(
      'a',
      { class: 'file-card', href: url, target: '_blank', rel: 'noopener', title: `Open ${a.name || a.path}` },
      h('span', { class: 'fc-icon' }, extOf(a.name || a.path).slice(0, 4)),
      h('span', { class: 'fc-text' }, h('span', { class: 'fc-name' }, a.name || a.path), h('span', { class: 'fc-meta' }, meta)),
      h(
        'span',
        {
          class: 'fc-save',
          role: 'button',
          'aria-label': `Save ${a.name || 'file'}`,
          'data-tip': 'Save',
          onclick: (e) => {
            e.preventDefault();
            e.stopPropagation();
            const link = h('a', { href: url, download: a.name || '' });
            document.body.append(link);
            link.click();
            link.remove();
          },
        },
        icon('download', 16)
      )
    );
  }
  function attachmentMsg(m) {
    const a = Object.assign({}, m.meta || {});
    if (!a.name && a.path) a.name = a.path.split(/[\\/]/).pop();
    if (isImage(a)) return [imageRow([a])];
    return [fileCard(a)];
  }
  function openLightbox(items, start = 0) {
    let i = start;
    const img = h('img', { alt: '' });
    const name = h('div', { class: 'lightbox-name' });
    const paint = () => {
      img.src = items[i].url;
      img.alt = items[i].name;
      name.textContent = items.length > 1 ? `${items[i].name} · Image ${i + 1} of ${items.length}` : items[i].name;
    };
    const box = h('div', { class: 'lightbox', onclick: () => close() }, img, name);
    const onKey = (e) => {
      if (items.length < 2) return;
      if (e.key === 'ArrowRight') i = (i + 1) % items.length;
      else if (e.key === 'ArrowLeft') i = (i - 1 + items.length) % items.length;
      else return;
      paint();
    };
    document.addEventListener('keydown', onKey);
    const close = openModal(box, { cls: 'dark-overlay', onClose: () => document.removeEventListener('keydown', onKey) });
    paint();
  }

  // ── widget (question with quick replies)
  function widgetCard(m) {
    const meta = m.meta || {};
    const opts = Array.isArray(meta.options) ? meta.options : [];
    const answered = meta.answer != null;
    const card = h('div', { class: 'card widget' + (answered ? ' answered' : ''), role: 'group', 'aria-label': meta.question || 'Question' });
    card.append(h('div', { class: 'card-title' }, meta.question || m.text || 'Question'));
    if (meta.screen) {
      card.classList.add('bs-ask');
      card.append(
        h('div', { class: 'card-sub' }, answered ? 'You took over the Bot’s screen.' : 'The Bot needs you to do this on its screen. Open it, take control, then choose I’m done.'),
        h('button', { class: 'btn' + (answered ? '' : ' primary') + ' bs-open', onclick: () => openScreen(m.bot_id, { control: !answered }) }, icon('monitor', 15), answered ? 'View screen' : 'Open screen')
      );
    }
    const list = h('div', { class: 'opts' });
    const buttons = [];
    let busy = false;
    const answer = async (value) => {
      if (busy || answered) return;
      busy = true;
      buttons.forEach((x) => (x.disabled = true));
      try {
        const um = await api(`/api/messages/${encodeURIComponent(m.id)}/answer`, { method: 'POST', body: { value } });
        const nm = Object.assign({}, m, { meta: Object.assign({}, meta, { answer: value }) });
        upsertMessage(nm);
        if (um && um.id) upsertMessage(um);
        S.newDiv = null;
      } catch (e) {
        busy = false;
        buttons.forEach((x) => (x.disabled = false));
        fail(e);
      }
    };
    opts.forEach((o, i) => {
      const chosen = answered && String(meta.answer) === String(o);
      const b = h(
        'button',
        { class: 'opt' + (chosen ? ' chosen' : ''), disabled: answered, onclick: () => answer(o), 'data-opt': String(i) },
        h('span', { class: 'opt-key' }, chosen ? icon('check', 13, '', 2.4) : String.fromCharCode(65 + i)),
        h('span', null, o)
      );
      buttons.push(b);
      list.append(b);
    });
    if (meta.allow_custom && !answered) {
      const other = h(
        'button',
        {
          class: 'opt',
          onclick: () => {
            other.hidden = true;
            customRow.hidden = false;
            field.focus();
          },
        },
        h('span', { class: 'opt-key' }, icon('pencil', 12, '', 2)),
        h('span', null, 'Something else')
      );
      buttons.push(other);
      const field = h('input', { placeholder: 'Type your answer', 'aria-label': 'Your answer', maxlength: '500' });
      const go = h('button', { class: 'c-send', 'data-mode': 'send', type: 'button', 'aria-label': 'Send answer' }, icon('arrowUp', 16, 'g-send', 2.2));
      const submit = () => {
        const v = field.value.trim();
        if (v) answer(v);
      };
      go.addEventListener('click', submit);
      field.addEventListener('keydown', (e) => {
        e.stopPropagation();
        if (e.key === 'Enter') submit();
        if (e.key === 'Escape') {
          customRow.hidden = true;
          other.hidden = false;
        }
      });
      const customRow = h('div', { class: 'custom-row', hidden: true }, field, go);
      buttons.push(go);
      list.append(other, customRow);
    }
    card.append(list);
    if (answered && !opts.some((o) => String(o) === String(meta.answer))) card.append(h('div', { class: 'answer-note' }, `You answered: ${meta.answer}`));
    card._answer = answer;
    card._opts = opts;
    return card;
  }

  // Secure form (secure_form.rs): what the user types goes straight into the page on the Bot's
  // screen. It is never shown to the Bot or kept in the chat.
  function formCard(m) {
    const meta = m.meta || {};
    const pending = m.status === 'pending';
    const card = h('div', { class: 'card widget sform' + (pending ? '' : ' answered'), role: 'group', 'aria-label': meta.title || 'Secure form' });
    card.append(h('div', { class: 'card-title sform-title' }, icon('lock', 15), h('span', null, meta.title || 'Secure form')));
    if (meta.reason) card.append(h('div', { class: 'card-sub' }, meta.reason));
    if (!pending) {
      let done;
      if (m.status === 'dismissed') done = 'You closed this form.';
      else if (m.status === 'replaced') done = 'The Bot asked again with a newer form.';
      else {
        done = `Filled in ${(meta.filled || []).join(', ')} on ${meta.host}.`;
        if (meta.saved === 'added') done += ' Saved to the Vault.';
        else if (meta.saved === 'updated') done += ' Updated the login in the Vault.';
        else if (typeof meta.saved === 'string') done += ` Couldn’t save it to the Vault (${meta.saved.replace(/^not saved: /, '')}).`;
      }
      card.append(h('div', { class: 'answer-note' }, done));
      return card;
    }
    card.append(h('div', { class: 'card-sub sform-site' }, icon('globe', 13), h('span', null, `Fills on ${meta.host}. The Bot never sees what you type.`)));
    const inputs = {};
    const body = h('div', { class: 'sform-fields' });
    for (const f of meta.fields || []) {
      const type = f.type === 'password' ? 'password' : f.type === 'email' ? 'email' : f.type === 'tel' ? 'tel' : 'text';
      const inp = h('input', { class: 'field' + (f.type === 'password' ? ' mono' : ''), type, autocomplete: f.type === 'code' ? 'one-time-code' : 'off', inputmode: f.type === 'code' ? 'numeric' : null, spellcheck: 'false', 'aria-label': f.label });
      inp.addEventListener('keydown', (e) => {
        e.stopPropagation();
        if (e.key === 'Enter') submit();
      });
      inputs[f.id] = inp;
      body.append(h('label', { class: 'f-label' }, f.label), inp);
    }
    card.append(body);
    let save = null;
    if (meta.save_available) {
      save = h('input', { type: 'checkbox', checked: true });
      card.append(h('label', { class: 'sform-save' }, save, h('span', null, 'Securely save to the Vault')));
    }
    const err = h('div', { class: 'f-err', role: 'alert', hidden: true });
    const fillBtn = h('button', { class: 'btn primary' }, 'Fill in');
    const closeBtn = h('button', { class: 'btn' }, 'Not now');
    card.append(err, h('div', { class: 'sform-actions' }, closeBtn, fillBtn));
    let busy = false;
    const send = async (body) => {
      if (busy) return;
      busy = true;
      fillBtn.disabled = closeBtn.disabled = true;
      err.hidden = true;
      try {
        const nm = await api(`/api/messages/${encodeURIComponent(m.id)}/form`, { method: 'POST', body });
        Object.values(inputs).forEach((i) => (i.value = ''));
        upsertMessage(nm);
      } catch (e) {
        busy = false;
        fillBtn.disabled = closeBtn.disabled = false;
        err.textContent = e.message;
        err.hidden = false;
      }
    };
    const submit = () => {
      const values = {};
      for (const [id, inp] of Object.entries(inputs)) if (inp.value) values[id] = inp.value;
      if (!Object.keys(values).length) {
        err.textContent = 'Fill in at least one box';
        err.hidden = false;
        return;
      }
      send({ values, save: !!(save && save.checked) });
    };
    fillBtn.addEventListener('click', submit);
    closeBtn.addEventListener('click', () => send({ dismiss: true }));
    return card;
  }

  // ── bot screens: each Bot has its own Linux screen (Xvfb + a browser) that you can watch and take over
  async function loadScreenStatus() {
    try {
      S.screenStatus = await api('/api/screens/status');
      if (S.screenStatus.setup_running && !S.screenSetup) S.screenSetup = { state: 'running' };
      S.screensOn = new Set(S.screenStatus.running || []);
    } catch {
      S.screenStatus = null;
    }
    renderScreenBtn();
    if (S.settingsUI && S.settingsUI.tab === 'computer') rerenderSettings();
  }
  function renderScreenBtn() {
    const btn = $('#screenBtn');
    if (!btn) return;
    const b = curBot();
    btn.hidden = !b || isGroup(b) || !(S.screenStatus && S.screenStatus.supported);
    const on = !!b && S.screensOn.has(b.id);
    btn.classList.toggle('live', on);
    btn.dataset.tip = on ? 'Bot’s screen (on)' : 'Bot’s screen';
    btn.setAttribute('aria-label', on ? 'Bot’s screen, on' : 'Bot’s screen');
  }
  function screensRow() {
    const st = S.screenStatus;
    if (!st || !st.supported) return null;
    const busy = S.screenSetup && S.screenSetup.state === 'running';
    const failed = S.screenSetup && S.screenSetup.state === 'error' ? S.screenSetup.error : '';
    let sub;
    let ctl;
    if (busy) {
      sub = 'Installing the screen programs and a browser in Linux. This takes a few minutes.';
      ctl = h('span', { class: 'set-status' }, spinner('xs'), 'Setting up…');
    } else if (st.ready) {
      sub = `Each Bot gets its own screen with a browser (in ${st.where || 'Linux'}). Watch it or take control from the screen button at the top of a chat.`;
      ctl = h('span', { class: 'set-status ok' }, icon('check', 14, '', 2.4), 'Ready');
    } else {
      const missing = (st.missing || []).concat(st.browser ? [] : ['a browser']);
      sub = `Bots need a few Linux programs to get their own screen${missing.length ? ` (missing: ${missing.join(', ')})` : ''}.`;
      ctl = btn('Set up', async () => {
        S.screenSetup = { state: 'running' };
        rerenderSettings();
        try {
          await api('/api/screens/setup', { method: 'POST' });
        } catch (e) {
          S.screenSetup = { state: 'error', error: (e && e.message) || String(e) };
          rerenderSettings();
        }
      });
    }
    return h(
      'div',
      null,
      row('Bot screens', ctl, sub, '', 'bot-screens'),
      failed ? h('pre', { class: 'set-log err', role: 'alert' }, failed) : null
    );
  }

  // The live viewer. control:true starts with "Take control" on (used by the hand-over card).
  function openScreen(botId, { control = false } = {}) {
    if (S.screenUI) S.screenUI.close();
    const bot = S.bots.get(botId);
    const name = bot ? bot.name : 'Bot';
    let rfb = null;
    let closing = false;
    let ctl = false;
    const status = h('span', { class: 'bs-status' }, 'Starting the screen…');
    const ctlBtn = h('button', { class: 'btn primary', disabled: true, onclick: () => setControl(!ctl) });
    const reBtn = h('button', { class: 'btn', hidden: true, onclick: () => connect() }, icon('refresh', 15), 'Reconnect');
    const stopBtn = h(
      'button',
      {
        class: 'ghost-btn',
        'data-tip': 'Turn off the screen (its browser closes; logins are kept)',
        'aria-label': 'Turn off the screen',
        onclick: () => {
          // Closing the browser cleanly takes a few seconds; the toolbar dot updates when it's done.
          close();
          api(`/api/bots/${encodeURIComponent(botId)}/screen/stop`, { method: 'POST' }).catch(fail);
        },
      },
      icon('power', 17)
    );
    const closeBtn = h('button', { class: 'ghost-btn', 'aria-label': 'Close', 'data-tip': 'Close', onclick: () => close() }, icon('x', 17, '', 2.2));
    const msg = h('div', { class: 'bs-msg' }, spinner('lg'));
    const stage = h('div', { class: 'bs-stage' }, msg);
    const hint = h('div', { class: 'bs-hint' });
    const box = h(
      'div',
      { class: 'dialog bs-dlg', role: 'dialog', 'aria-modal': 'true', 'aria-label': `${name}’s screen` },
      h('div', { class: 'bs-head' }, h('div', { class: 'bs-title' }, h('h2', { class: 'dialog-title' }, `${name}’s screen`), status), h('div', { class: 'bs-actions' }, reBtn, ctlBtn, stopBtn, closeBtn)),
      stage,
      hint
    );
    const closeModal = openModal(box, { cls: 'bs-overlay', onClose: () => cleanup() });
    S.screenUI = {
      botId,
      close: () => close(),
      stopped: () => {
        if (closing) return;
        showMsg('The screen was turned off.', true);
      },
    };
    function close() {
      closeModal();
    }
    function cleanup() {
      closing = true;
      if (rfb) rfb.disconnect();
      rfb = null;
      if (S.screenUI && S.screenUI.botId === botId) S.screenUI = null;
    }
    function setStatus(text, kind = '') {
      status.textContent = text;
      status.dataset.kind = kind;
    }
    function showMsg(text, retry) {
      msg.textContent = '';
      msg.append(h('div', null, text));
      if (retry) msg.append(h('button', { class: 'btn', onclick: () => connect() }, icon('refresh', 15), 'Start it again'));
      msg.hidden = false;
      ctlBtn.disabled = true;
      hint.textContent = '';
      setStatus('Off', 'off');
    }
    function setControl(on) {
      ctl = on;
      ctlBtn.textContent = '';
      ctlBtn.append(icon(on ? 'check' : 'pointer', 15, '', 2.2), on ? 'Hand back' : 'Take control');
      ctlBtn.classList.toggle('primary', !on);
      stage.classList.toggle('controlling', on);
      if (rfb) {
        rfb.viewOnly = !on;
        if (on) rfb.focus();
      }
      if (!rfb || !msg.hidden) return; // not connected yet: connect() shows the status
      setStatus(on ? 'You’re in control' : 'Watching', on ? 'ctl' : 'live');
      hint.textContent = on
        ? 'Your mouse and keyboard now go to the Bot’s screen. Choose Hand back when you’re done.'
        : 'You’re watching. The Bot keeps working; choose Take control to use the screen yourself.';
    }
    async function connect() {
      reBtn.hidden = true;
      msg.textContent = '';
      msg.append(spinner('lg'));
      msg.hidden = false;
      setStatus('Starting the screen…');
      if (rfb) {
        rfb.disconnect();
        rfb = null;
      }
      let info;
      let RFB;
      try {
        info = await api(`/api/bots/${encodeURIComponent(botId)}/screen/start`, { method: 'POST' });
        ({ default: RFB } = await import('/vendor/novnc/core/rfb.js'));
      } catch (e) {
        if (closing) return;
        const text = (e && e.message) || 'The screen couldn’t start.';
        msg.textContent = '';
        msg.append(h('div', null, text));
        if (/set up/i.test(text))
          msg.append(
            h('button', { class: 'btn primary', onclick: () => { close(); openSettings('computer'); } }, 'Open Settings')
          );
        else msg.append(h('button', { class: 'btn', onclick: () => connect() }, icon('refresh', 15), 'Try again'));
        setStatus('Off', 'off');
        return;
      }
      if (closing) return;
      S.screensOn.add(botId);
      renderScreenBtn();
      const u = new URL(S.apiBase);
      u.protocol = u.protocol === 'https:' ? 'wss:' : 'ws:';
      u.pathname = `/api/bots/${encodeURIComponent(botId)}/screen/ws`;
      const r = new RFB(stage, u.toString(), { credentials: { password: info.password }, wsProtocols: ['binary', 'openbot.auth.' + (S.apiToken || '')] });
      rfb = r;
      r.scaleViewport = true;
      r.resizeSession = false;
      r.background = 'transparent';
      r.viewOnly = !ctl;
      r.addEventListener('connect', () => {
        if (rfb !== r) return;
        msg.hidden = true;
        ctlBtn.disabled = false;
        setControl(ctl);
      });
      r.addEventListener('disconnect', (ev) => {
        if (rfb !== r || closing) return;
        rfb = null;
        ctlBtn.disabled = true;
        msg.textContent = '';
        msg.append(h('div', null, ev.detail && ev.detail.clean ? 'The screen closed.' : 'Lost the connection to the screen.'));
        msg.hidden = false;
        reBtn.hidden = false;
        hint.textContent = '';
        setStatus('Disconnected', 'off');
      });
      r.addEventListener('credentialsrequired', () => r.sendCredentials({ password: info.password }));
      r.addEventListener('securityfailure', () => {
        if (rfb === r) setStatus('Couldn’t sign in to the screen', 'off');
      });
    }
    setControl(control);
    ctlBtn.disabled = true;
    setStatus('Starting the screen…');
    connect();
  }

  // ── visualization (sandboxed page made by a bot)
  function vizCard(m) {
    const meta = m.meta || {};
    const src = meta.url ? (/^https?:/.test(meta.url) ? meta.url : S.apiBase + meta.url) : '';
    const frame = h('iframe', { class: 'viz-frame', src, sandbox: 'allow-scripts allow-popups', loading: 'lazy', title: meta.title || 'Visualization', referrerpolicy: 'no-referrer' });
    return h(
      'div',
      { class: 'card viz' },
      h(
        'div',
        { class: 'viz-head' },
        icon('chart', 16, '', 1.8),
        h('span', { class: 'viz-title' }, meta.title || m.text || 'Page'),
        h('button', { class: 'ghost-btn', 'aria-label': 'Open in a new window', 'data-tip': 'Open in a new window', onclick: () => window.open(src, '_blank', 'noopener') }, icon('external', 16))
      ),
      frame
    );
  }
  // pages can report their height: parent.postMessage({type:'openbot:height', height}, '*')
  window.addEventListener('message', (e) => {
    const d = e.data;
    if (!d || typeof d !== 'object' || !(d.type === 'openbot:height' || d.type === 'resize') || typeof d.height !== 'number') return;
    for (const f of $$('.viz-frame')) {
      if (f.contentWindow === e.source) {
        const pin = S.pinned;
        f.style.height = clamp(Math.ceil(d.height), 120, 600) + 'px';
        if (pin) msgsEl.scrollTop = msgsEl.scrollHeight;
      }
    }
  });

  // ── approval card (Allow / Always allow / Deny)
  const APPROVAL_VERB = {
    Bash: ['terminal', 'Terminal', 'run a command'],
    PowerShell: ['terminal', 'Terminal', 'run a command'],
    Write: ['file', 'Files', 'create a file'],
    Edit: ['pencil', 'Files', 'edit a file'],
    MultiEdit: ['pencil', 'Files', 'edit a file'],
    NotebookEdit: ['pencil', 'Files', 'edit a notebook'],
    Read: ['file', 'Files', 'read a file'],
    WebFetch: ['globe', 'Web', 'open a web page'],
    WebSearch: ['globe', 'Web', 'search the web'],
    mcp__openbot__delete_bot: ['trash', 'Bots', 'delete a Bot'],
    mcp__openbot__email_send: ['mail', 'Email', 'send an email'],
    mcp__openbot__email_update: ['trash', 'Email', 'move email to the trash'],
    mcp__openbot__calendar_create: ['calendar', 'Calendar', 'add an event and invite people'],
    mcp__openbot__calendar_update: ['calendar', 'Calendar', 'change an event'],
    mcp__openbot__calendar_delete: ['calendar', 'Calendar', 'delete an event'],
    mcp__openbot__calendar_respond: ['calendar', 'Calendar', 'reply to an invitation'],
  };
  // Approval card (settings-and-bots.md 2.3.1)
  const approvalAnswers = new Map(); // approval_id → 'allow' | 'always' | 'deny' | 'expired'
  function cutMiddle(text, max = 340) {
    if (text.length <= max) return text;
    const keep = Math.floor((max - 30) / 2);
    const omitted = text.length - keep * 2;
    return text.slice(0, keep) + `...[${omitted} chars omitted]...` + text.slice(text.length - keep);
  }
  // What an email / calendar change looks like, on its approval card.
  function approvalPreview(x) {
    const line = (k, v) => (v && (!Array.isArray(v) || v.length) ? h('div', { class: 'mail-line' }, h('span', { class: 'mail-k' }, k), h('span', { class: 'mail-v' }, Array.isArray(v) ? v.join(', ') : v)) : null);
    if (x.kind === 'email')
      return h(
        'div',
        { class: 'card-well mail' },
        line('From', x.account),
        line('To', x.to),
        line('Cc', x.cc),
        line('Bcc', x.bcc),
        line('Subject', x.subject),
        line('Files', x.attachments),
        h('div', { class: 'mail-body' }, x.body || '')
      );
    if (x.kind === 'email-trash') return h('div', { class: 'card-well mail' }, line('Account', x.account), ...(x.subjects || []).map((t) => h('div', { class: 'mail-line' }, h('span', { class: 'mail-v' }, t))));
    if (x.kind === 'event')
      return h('div', { class: 'card-well mail' }, line('Calendar', x.account), line('Event', x.title), line('When', x.when), line('Where', x.location), line('Guests', x.attendees), x.description ? h('div', { class: 'mail-body' }, x.description) : null);
    return null;
  }
  // ── calendar card: an event a Bot added or changed, or wants to send (inside its approval card)
  const CAL_ACTION = {
    create: { allow: 'Send invite', deny: 'Don’t send', done: 'Invite sent', added: 'Added to your calendar' },
    update: { allow: 'Send update', deny: 'Don’t send', done: 'Update sent', added: 'Updated in your calendar' },
    delete: { allow: 'Delete event', deny: 'Keep it', done: 'Deleted' },
    respond: { allow: 'Send reply', deny: 'Don’t send', done: 'Reply sent' },
  };
  const REPLY_WORD = { yes: 'Yes', no: 'No', maybe: 'Maybe', waiting: 'Waiting' };
  const dayOf = (iso) => {
    const [y, mo, d] = String(iso || '').split('-').map(Number);
    return y ? new Date(y, mo - 1, d) : null;
  };
  const calDate = (d) => d.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' });
  const calTime = (d) => d.toLocaleTimeString(undefined, d.getMinutes() ? { hour: 'numeric', minute: '2-digit' } : { hour: 'numeric' });
  function calWhen(x) {
    if (x.all_day) {
      const a = dayOf(x.first_day);
      const b = dayOf(x.last_day);
      if (!a) return [x.when || ''];
      return !b || +a === +b ? [calDate(a), 'All day'] : [`${calDate(a)} – ${calDate(b)}`, 'All day'];
    }
    const a = new Date(x.start);
    const b = new Date(x.end);
    const sameDay = a.toDateString() === b.toDateString();
    return sameDay ? [calDate(a), `${calTime(a)} – ${calTime(b)}`] : [`${calDate(a)}, ${calTime(a)}`, `${calDate(b)}, ${calTime(b)}`];
  }
  // A 4-hour view of the day around the event, with the user's other events beside it.
  function calTimeline(x) {
    const HOUR = 3600000;
    const ROW = 30;
    const day = x.all_day ? dayOf(x.first_day) || new Date(x.start) : new Date(x.start);
    const midnight = new Date(day.getFullYear(), day.getMonth(), day.getDate()).getTime();
    const firstHour = x.all_day ? 8 : Math.min(20, Math.max(0, new Date(x.start).getHours() - 1));
    const from = midnight + firstHour * HOUR;
    const to = from + 4 * HOUR;
    const timed = (x.day_events || []).filter((e) => !e.all_day && e.end > from && e.start < to).map((e) => ({ ...e }));
    const allDay = (x.day_events || []).filter((e) => e.all_day);
    if (!x.all_day) timed.push({ title: x.title || '(No title)', start: x.start, end: x.end, proposed: true });
    timed.sort((a, b) => a.start - b.start || b.end - a.end);
    const lanes = [];
    for (const e of timed) {
      let i = lanes.findIndex((end) => end <= e.start);
      if (i < 0) i = lanes.push(0) - 1;
      lanes[i] = e.end;
      e.lane = i;
    }
    const n = Math.max(1, lanes.length);
    const grid = h('div', { class: 'cal-grid', style: { height: ROW * 4 + 'px' } });
    for (let i = 0; i < 4; i++) {
      grid.append(h('div', { class: 'cal-hour', style: { top: i * ROW + 'px' } }, h('span', null, calTime(new Date(from + i * HOUR)))));
    }
    for (const e of timed) {
      const top = Math.max(0, ((e.start - from) / HOUR) * ROW);
      const bottom = Math.min(ROW * 4, ((e.end - from) / HOUR) * ROW);
      const height = Math.max(ROW / 2, bottom - top);
      grid.append(
        h(
          'div',
          { class: 'cal-block' + (e.proposed ? ' proposed' : ''), style: { top: top + 'px', height: height + 'px', left: `calc(52px + (100% - 52px) * ${e.lane / n})`, width: `calc((100% - 52px) / ${n} - 3px)` }, title: e.title },
          h('span', null, e.title || '(No title)')
        )
      );
    }
    const wrap = h('div', { class: 'cal-day' });
    const strip = [...allDay.map((e) => e.title || '(No title)'), ...(x.all_day ? [x.title || '(No title)'] : [])];
    if (strip.length) wrap.append(h('div', { class: 'cal-allday' }, ...strip.map((t, i) => h('span', { class: 'cal-chip' + (x.all_day && i === strip.length - 1 ? ' proposed' : '') }, t))));
    wrap.append(grid);
    return wrap;
  }
  function calBody(x, open) {
    const guests = (x.guests || []).filter((g) => !g.organizer || !g.self);
    const counts = {};
    for (const g of guests) counts[g.response] = (counts[g.response] || 0) + 1;
    const summary = ['yes', 'no', 'maybe', 'waiting'].filter((k) => counts[k]).map((k) => `${counts[k]} ${REPLY_WORD[k]}`).join(', ');
    const [d1, d2] = calWhen(x);
    const body = h('div', { class: 'cal-body' });
    const row = (label, ...v) => h('div', { class: 'cal-row' }, h('span', { class: 'cal-k' }, label), h('div', { class: 'cal-v' }, ...v));
    const details = h('div', { class: 'cal-details', hidden: !open });
    const toggle = h('button', { class: 'cal-toggle', 'aria-expanded': open ? 'true' : 'false', onclick: () => { details.hidden = !details.hidden; toggle.setAttribute('aria-expanded', details.hidden ? 'false' : 'true'); toggle.classList.toggle('open', !details.hidden); } }, 'Details', icon('chevronDown', 12, '', 2));
    if (open) toggle.classList.add('open');
    body.append(row('When', h('span', { class: 'cal-chip' }, d1), d2 ? h('span', { class: 'cal-chip' }, d2) : null, toggle));
    if (x.repeat) body.append(row('Repeats', h('span', null, x.repeat.charAt(0).toUpperCase() + x.repeat.slice(1))));
    details.append(calTimeline(x));
    body.append(details);
    if (guests.length) {
      const list = h(
        'div',
        { class: 'cal-guests' },
        ...guests.map((g) =>
          h(
            'div',
            { class: 'cal-guest' },
            h('span', { class: 'cal-gname' }, g.name || g.email, g.self ? h('span', { class: 'cal-note' }, ' (You)') : null, g.optional ? h('span', { class: 'cal-note' }, ' · Optional') : null),
            h('span', { class: 'cal-reply ' + g.response }, REPLY_WORD[g.response] || g.response)
          )
        )
      );
      body.append(row('Guests', h('div', null, h('div', { class: 'cal-sum' }, summary), list)));
    }
    if (x.removed && x.removed.length) body.append(row('Removed', h('span', null, x.removed.join(', '))));
    if (x.location) body.append(row('Location', h('span', null, x.location)));
    if (x.video) body.append(row('Video call', h('a', { href: x.video, target: '_blank', rel: 'noopener noreferrer' }, x.video.replace(/^https?:\/\//, ''))));
    if (x.organizer) body.append(row('Organizer', h('span', null, x.organizer === x.account ? `${x.organizer} (You)` : x.organizer)));
    if (x.action === 'respond') body.append(row('Your reply', h('span', { class: 'cal-reply ' + x.response }, REPLY_WORD[x.response] || x.response), x.comment ? h('span', { class: 'cal-note' }, ` “${x.comment}”`) : null));
    if (x.description && open) body.append(h('div', { class: 'cal-desc' }, x.description));
    return body;
  }
  function calHead(x) {
    const src = x.calendar ? `Calendar · ${x.calendar}` : 'Calendar';
    return [h('div', { class: 'card-src' }, icon('calendar', 14), h('span', null, src)), h('div', { class: 'card-title' }, x.title || '(No title)')];
  }
  // An event the Bot added or changed (no approval needed).
  function calendarCard(m) {
    const x = m.meta || {};
    const act = CAL_ACTION[x.action] || CAL_ACTION.create;
    const openIn = x.link ? h('a', { class: 'cal-open', href: x.link, target: '_blank', rel: 'noopener noreferrer' }, x.provider === 'microsoft' ? 'Open in Outlook' : 'Open in Google Calendar', icon('external', 12, '', 2)) : null;
    return h('div', { class: 'card cal' }, ...calHead(x), calBody(x, false), h('div', { class: 'card-settled ok' }, icon('check', 13, '', 2.2), h('span', null, act.added || act.done), openIn));
  }
  function approvalCard(m) {
    const meta = m.meta || {};
    const b = ownerOf(m);
    const botName = b ? b.name : 'Your Bot';
    const tool = meta.tool_name || '';
    const shell = tool === 'Bash' || tool === 'PowerShell';
    const mcp = /^mcp__([^_]+(?:_[^_]+)*)__(.+)$/.exec(tool);
    const [ic, , verb] = APPROVAL_VERB[tool] || (mcp ? ['bolt', '', `use ${mcp[2].replace(/_/g, ' ')}`] : ['bolt', '', `use ${tool || 'a tool'}`]);
    const source = shell ? 'Terminal' : APPROVAL_VERB[tool] ? APPROVAL_VERB[tool][1] : mcp ? mcp[1].replace(/[_-]/g, ' ') : 'Auto-review';
    const input = meta.input && typeof meta.input === 'object' ? meta.input : null;
    const rule = (meta.proposed_rule || '').trim();
    let well = null;
    let full = '';
    if (input) {
      const v = input.command || input.file_path || input.notebook_path || input.url || input.query || input.pattern || null;
      if (typeof v === 'string') well = h('div', { class: 'card-well mono' }, v.length > 340 ? cutMiddle(v) : v);
      full = typeof v === 'string' && Object.keys(input).length === 1 ? v : JSON.stringify(input, null, 2);
    }
    const cal = input && input.kind === 'event' && input.start != null ? input : null;
    if (input && input.kind) {
      well = cal ? calBody(cal, m.status === 'pending') : approvalPreview(input);
      full = '';
    }
    if (!well && (meta.summary || m.text)) well = h('div', { class: 'card-well' }, meta.summary || m.text);
    const reason = (meta.reason || '').trim();
    const local = approvalAnswers.get(meta.approval_id);
    let status = m.status;
    if (status === 'pending' && local === 'expired') status = 'expired';
    const decided = meta.decision || (status === 'allowed' ? local || 'allow' : status === 'denied' ? 'deny' : null);
    const card = h(
      'div',
      { class: 'card approval ' + status },
      h('div', { class: 'card-src' }, icon(shell ? 'terminal' : ic, 14), h('span', null, source)),
      h('div', { class: 'card-title' }, cal ? cal.title || '(No title)' : `${botName} wants to ${verb}`),
      cal && h('div', { class: 'card-desc' }, `${botName} wants to ${verb}`),
      reason && h('div', { class: 'card-caution' }, icon('alert', 14), h('span', null, reason)),
      well,
      full && full !== (well && well.textContent) ? h('details', { class: 'card-full' }, h('summary', null, 'View the full request'), h('pre', null, full)) : null
    );
    if (status === 'pending') {
      const buttons = [];
      const errLine = h('div', { class: 'card-error', role: 'alert', hidden: true }, 'Your answer didn’t go through. Check your connection and try again.');
      const decide = async (decision) => {
        buttons.forEach((x) => (x.disabled = true));
        errLine.hidden = true;
        try {
          await api(`/api/approvals/${encodeURIComponent(meta.approval_id)}`, { method: 'POST', body: { decision } });
          approvalAnswers.set(meta.approval_id, decision);
          upsertMessage(Object.assign({}, m, { status: decision === 'deny' ? 'denied' : 'allowed' }));
        } catch (e) {
          if (e && e.status === 404) {
            approvalAnswers.set(meta.approval_id, 'expired');
            upsertMessage(Object.assign({}, m));
            return;
          }
          buttons.forEach((x) => (x.disabled = false));
          errLine.hidden = false;
        }
      };
      if (rule) card.append(h('div', { class: 'rule-notice' }, h('div', { class: 'rule-notice-title' }, 'Adding to your Allow rules'), h('div', { class: 'rule-notice-text' }, rule)));
      const calAct = cal && CAL_ACTION[cal.action];
      buttons.push(h('button', { class: 'btn primary', onclick: () => decide('allow') }, calAct ? calAct.allow : rule ? 'Allow once' : 'Allow'));
      if (rule) buttons.push(h('button', { class: 'btn', onclick: () => decide('always') }, 'Always allow'));
      buttons.push(h('button', { class: 'btn ghost', onclick: () => decide('deny') }, calAct ? calAct.deny : shell ? 'Skip' : 'Deny'));
      card.append(h('div', { class: 'card-actions' }, buttons), errLine);
    } else {
      const calAct = cal && CAL_ACTION[cal.action];
      const label = status === 'skipped' ? 'Skipped' : status === 'expired' ? 'Expired' : decided === 'always' ? 'Always allowed' : decided === 'deny' ? (calAct ? 'Not sent' : 'Denied') : status === 'allowed' ? (calAct ? calAct.done : 'Allowed once') : 'Status unavailable';
      const ok = status === 'allowed';
      card.append(
        h(
          'div',
          { class: 'card-settled' },
          h('span', { class: 'settled-pill ' + (ok ? 'ok' : status === 'denied' ? 'no' : '') }, ok ? icon('check', 13, '', 2.2) : status === 'denied' ? icon('x', 13, '', 2.2) : null, label),
          status === 'skipped' ? h('span', { class: 'settled-note' }, 'You moved on without answering') : decided === 'always' && rule ? h('span', { class: 'settled-note' }, `Allowed from now on: ${rule}`) : decided === 'deny' ? h('span', { class: 'settled-note' }, `${botName} won’t do this`) : null
        )
      );
    }
    return card;
  }
  // Under a message sent while the Bot was busy: "Queued · Send now", or "Not sent" after Stop.
  function queuedRow(m) {
    const meta = m.meta || {};
    if (meta.not_sent) return h('div', { class: 'q-row' }, h('span', { class: 'q-label' }, 'Not sent. The Bot was stopped first.'));
    if (!meta.queued) return null;
    const btn = h('button', { class: 'q-send' }, 'Send now');
    btn.onclick = async () => {
      btn.disabled = true;
      try {
        await api(`/api/bots/${m.bot_id}/steer`, { method: 'POST', body: { message_id: m.id } });
      } catch (e) {
        btn.disabled = false;
        fail(e);
      }
    };
    return h('div', { class: 'q-row' }, h('span', { class: 'q-label' }, 'Queued. The Bot will read this when it’s done.'), btn);
  }
  // Buttons only on the newest error of the chat (older ones are history).
  function isLastError(m) {
    const list = S.msgs.get(m.bot_id) || [];
    for (let i = list.length - 1; i >= 0; i--) {
      const x = list[i];
      if (x.role === 'error') return x.id === m.id;
      if (x.role === 'user' || x.role === 'bot') return false;
    }
    return false;
  }
  function errorActions(m) {
    const row = h('div', { class: 'err-actions' });
    const busy = (on) => row.querySelectorAll('button').forEach((b) => (b.disabled = on));
    const act = (label, fn) =>
      h('button', {
        class: 'err-btn',
        onclick: async () => {
          busy(true);
          try {
            await fn();
          } catch (e) {
            busy(false); // only after a failure: a second click would run it twice
            fail(e);
          }
        },
      }, label);
    for (const a of m.meta.actions) {
      if (a === 'retry') row.append(act('Try again', () => api(`/api/bots/${m.bot_id}/retry`, { method: 'POST' })));
      if (a === 'new_conversation')
        row.append(
          act('Start a new conversation', async () => {
            await api(`/api/bots/${m.bot_id}/new-conversation`, { method: 'POST' });
            toast('New conversation started. Memory, files and routines are kept');
          })
        );
      if (a === 'edit')
        row.append(
          act('Edit and resend', async () => {
            const r = await api(`/api/bots/${m.bot_id}/rollback`, { method: 'POST' });
            if (S.current !== m.bot_id) return;
            input.value = r.text || '';
            S.pending = (r.attachments || []).map((x) => Object.assign({}, x));
            renderAttachRow();
            updateComposer();
            input.focus();
          })
        );
    }
    return row;
  }
  // A message one Bot passed to another: "Messaged School Bot" here, "Message from General Bot" there.
  function relayRow(m) {
    const x = m.meta || {};
    const other = S.bots.get(x.bot_id);
    const name = (other && other.name) || x.name || 'another Bot';
    const incoming = x.direction === 'from';
    const open = incoming ? !S.expanded.has(m.id) : S.expanded.has(m.id);
    const toggle = () => {
      if (S.expanded.has(m.id)) S.expanded.delete(m.id);
      else S.expanded.add(m.id);
      scheduleRender();
    };
    const dot = h('span', { class: 'relay-dot', style: { background: other ? inkOf(lookOf(other).color) : 'var(--icon-3)' } });
    const head = h(
      'div',
      { class: 'event-line relay-head', title: m.created_at ? fmtTime(m.created_at) : null },
      h('button', { class: 'relay-toggle', 'aria-expanded': open ? 'true' : 'false', onclick: toggle }, dot, h('span', null, incoming ? `Message from ${name}` : `Messaged ${name}`), icon('chevronDown', 12, open ? 'flip' : '', 2.2)),
      other ? h('button', { class: 'event-open', onclick: () => selectBot(other.id) }, 'Open ' + other.name) : null
    );
    const box = h('div', { class: 'relay' }, head);
    if (open) box.append(h('div', { class: 'relay-text' }, m.text || ''));
    return box;
  }
  // "@School Bot" in a Bot's reply becomes a button that opens that Bot.
  function mentionify(root) {
    const bots = realBots().filter((b) => b.name).sort((a, b) => b.name.length - a.name.length);
    if (!bots.length || !root.textContent.includes('@')) return;
    const re = new RegExp('@(' + bots.map((b) => b.name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|') + ')(?![\\w])', 'gi');
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, { acceptNode: (n) => (n.parentElement.closest('code, pre, a') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT) });
    const nodes = [];
    while (walker.nextNode()) nodes.push(walker.currentNode);
    for (const n of nodes) {
      const t = n.nodeValue;
      const frag = document.createDocumentFragment();
      let last = 0;
      let mm;
      re.lastIndex = 0;
      while ((mm = re.exec(t))) {
        const b = bots.find((x) => x.name.toLowerCase() === mm[1].toLowerCase());
        frag.append(t.slice(last, mm.index), h('button', { class: 'mention', onclick: () => selectBot(b.id) }, '@' + b.name));
        last = mm.index + mm[0].length;
      }
      if (!last) continue;
      frag.append(t.slice(last));
      n.replaceWith(frag);
    }
  }
  function eventLine(m) {
    const meta = m.meta || {};
    const kind = meta.kind || '';
    const ic = /routine/.test(kind) ? 'clock' : kind === 'bot-deleted' ? 'trash' : /^bot-/.test(kind) ? 'sparkle' : 'bolt';
    const el = h('div', { class: 'event-line', title: m.created_at ? fmtTime(m.created_at) : null }, icon(ic, 13), h('span', null, m.text || (kind || 'Event').replace(/-/g, ' ')));
    // Events about another Bot (one this Bot set up, changed, or handed a routine to) link to it.
    const other = meta.bot_id && meta.bot_id !== m.bot_id ? S.bots.get(meta.bot_id) : null;
    if (other) el.append(h('button', { class: 'event-open', onclick: () => (S.bots.has(other.id) ? selectBot(other.id) : toast('That Bot was deleted')) }, 'Open ' + other.name));
    return el;
  }
  function toolIconName(name = '') {
    const n = name.toLowerCase();
    if (n === 'bash' || n.includes('shell') || n.includes('powershell') || n.includes('terminal')) return 'terminal';
    if (['read', 'glob', 'grep', 'ls', 'notebookread'].includes(n)) return 'file';
    if (['edit', 'write', 'multiedit', 'notebookedit'].includes(n)) return 'pencil';
    if (n.startsWith('web')) return 'globe';
    return 'bolt';
  }
  function toolView(m) {
    const meta = m.meta || {};
    const open = S.expanded.has(m.id);
    const hasOut = !!(meta.output && String(meta.output).trim());
    const box = h('div', { class: 'tool-row' + (open ? ' open' : '') + (m.status === 'failed' ? ' failed' : '') + (m.status === 'stopped' || m.status === 'declined' ? ' stopped' : '') });
    box.append(
      h(
        'button',
        {
          class: 'tool-line',
          disabled: !hasOut,
          onclick: () => {
            if (S.expanded.has(m.id)) S.expanded.delete(m.id);
            else S.expanded.add(m.id);
            box.classList.toggle('open');
          },
        },
        m.status === 'running' ? spinner('xs') : icon(toolIconName(meta.name), 14),
        h('span', { class: 'tool-sum' }, meta.summary || m.text || meta.name || 'Tool'),
        m.status === 'stopped' ? h('span', { class: 'tool-note' }, 'stopped') : null,
        m.status === 'declined' ? h('span', { class: 'tool-note' }, 'not done') : null,
        hasOut ? icon('chevronRight', 12, 'chev', 2.2) : null
      )
    );
    if (hasOut) box.append(h('pre', { class: 'tool-out' }, String(meta.output)));
    return box;
  }

  // ── incoming messages
  function upsertMessage(m, created) {
    noteForNotify(m, created);
    const list = S.msgs.get(m.bot_id);
    if (!list) return;
    let idx = list.findIndex((x) => x.id === m.id);
    if (idx < 0 && m.role === 'user') {
      const t = list.findIndex((x) => x._tmp && x.text === m.text);
      if (t >= 0) {
        adoptRow(list[t].id, m.id);
        list.splice(t, 1);
        idx = -1;
      }
    }
    idx = list.findIndex((x) => x.id === m.id);
    const isNew = idx < 0;
    if (isNew) {
      // keep the list sorted by time
      let at = list.length;
      while (at > 0 && list[at - 1].created_at > m.created_at && !list[at - 1]._tmp) at--;
      list.splice(at, 0, m);
    } else list[idx] = m;
    if (m.bot_id !== S.current) return;
    if (m.role === 'user' && !m._tmp && S.newDiv) S.newDiv = null;
    if (isNew && m.role !== 'user' && isShown(m) && !S.pinned) {
      S.newCount++;
    }
    if (m.role === 'user' && isNew) S.pinned = true;
    scheduleRender();
  }
  // The server confirmed a message shown while sending: it keeps the same row, so it doesn't
  // slide in a second time.
  function adoptRow(tmpId, id) {
    const c = S.nodes.get(tmpId);
    if (c && !S.nodes.has(id)) {
      S.nodes.delete(tmpId);
      c.el.classList.remove('enter');
      S.nodes.set(id, c);
    }
    const sep = S.seps.get('t:' + tmpId);
    if (sep && !S.seps.has('t:' + id)) {
      S.seps.delete('t:' + tmpId);
      S.seps.set('t:' + id, sep);
    }
  }
  function removeMessage(botId, id) {
    const list = S.msgs.get(botId);
    if (!list) return;
    const i = list.findIndex((x) => x.id === id);
    if (i >= 0) list.splice(i, 1);
    if (botId === S.current) scheduleRender();
  }
  function noteForNotify(m, created) {
    const b = S.bots.get(m.bot_id);
    if (!b || !TAURI) return;
    // In a group: "School Bot: …" under the group's name.
    const who = isGroup(b) ? ownerOf(m) : null;
    if (who && who.id !== b.id) {
      const orig = m;
      m = Object.assign({}, orig, { text: `${who.name}: ${orig.text || ''}`, meta: Object.assign({}, orig.meta, orig.meta && orig.meta.summary ? { summary: `${who.name}: ${orig.meta.summary}` } : {}) });
    }
    if (m.role === 'bot' && m.status === 'done' && (m.text || '').trim()) maybeNotify(b, 'done', m.text, 'd:' + m.id);
    else if (m.role === 'attachment' && created) maybeNotify(b, 'done', m.text || `Sent ${(m.meta && m.meta.name) || 'a file'}`, 'd:' + m.id);
    else if (m.role === 'visualization' && created) maybeNotify(b, 'done', (m.meta && m.meta.title) || 'Made a page', 'd:' + m.id);
    else if (m.role === 'approval' && m.status === 'pending') maybeNotify(b, 'needs', (m.meta && m.meta.summary) || m.text, 'a:' + m.id);
    else if (m.role === 'widget' && m.meta && m.meta.answer == null) maybeNotify(b, 'needs', m.meta.question || m.text, 'w:' + m.id);
    else if (m.role === 'form' && m.status === 'pending') maybeNotify(b, 'needs', (m.meta && m.meta.title) || m.text, 'f:' + m.id);
  }

  // ───────────────────────────── mark read ─────────────────────────────
  const markReadSoon = debounce(() => {
    const b = curBot();
    if (!b || !(b.unread > 0) || document.hidden || !document.hasFocus()) return;
    b.unread = 0;
    renderSidebar();
    api(`/api/bots/${b.id}/read`, { method: 'POST' }).catch(() => {});
  }, 300);
  window.addEventListener('focus', markReadSoon);
  document.addEventListener('visibilitychange', markReadSoon);

  // ───────────────────────────── composer ─────────────────────────────
  const input = $('#input');
  const composerEl = $('#composer');
  let notice = null; // {text, err}
  function setNotice(text, err) {
    notice = text ? { text, err } : null;
    const n = $('#cNotice');
    n.hidden = !notice;
    n.textContent = notice ? notice.text : '';
    n.classList.toggle('err', !!(notice && notice.err));
    layoutComposer();
    if (notice) {
      clearTimeout(setNotice.t);
      setNotice.t = setTimeout(() => setNotice(null), 6000);
    }
  }
  // 44px pill ↔ expanded box (shell.md 6.2/6.3)
  function layoutComposer() {
    const hasText = input.value.length > 0;
    const replying = !!(S.reply && S.reply.botId === S.current);
    let exp = (S.composerExpanded && hasText) || input.value.includes('\n') || S.pending.length > 0 || !!notice || replying;
    if (!exp && hasText) {
      composerEl.classList.remove('expanded');
      input.style.height = 'auto';
      if (input.scrollHeight >= 39) exp = true;
    }
    S.composerExpanded = exp;
    composerEl.classList.toggle('expanded', exp);
    input.style.height = 'auto';
    const sh = input.scrollHeight;
    input.style.height = clamp(sh, 20, 120) + 'px';
    input.classList.toggle('scrolls', sh > 120);
    const H = exp ? $('#cContent').offsetHeight : 44;
    composerEl.style.height = H + 'px';
    const dock = $('#dock');
    dock.style.setProperty('--composer-h', H + 'px');
    const ub = $('#usageBanner');
    $('#chat').style.setProperty('--dock-h', H + 16 + (ub && !ub.hidden ? ub.offsetHeight + 8 : 0) + 'px');
  }
  function placeholderFor(b) {
    if (!b) return 'Ask anything, or drop a file';
    if (S.reply && S.reply.botId === b.id) return 'Reply…';
    if (S.pending.length && !input.value) return 'Add a message, or hit send';
    const list = S.msgs.get(b.id) || [];
    if (openWidget(list)) return 'Type your answer, or talk about something else';
    return `Message ${b.name}`;
  }
  function updateComposer() {
    const b = curBot();
    input.placeholder = placeholderFor(b);
    input.disabled = !b;
    input.spellcheck = S.prefs.spellcheck !== false;
    $('#attachBtn').disabled = !b || S.pending.length >= 6;
    const hasText = input.value.trim().length > 0;
    const ready = S.pending.some((p) => !p.uploading && !p.error);
    const btn = $('#sendBtn');
    let mode;
    if (b && (hasText || ready)) mode = 'send';
    else if (b && b.running) mode = 'stop';
    else mode = 'disabled';
    btn.dataset.mode = mode;
    btn.disabled = mode === 'disabled';
    const labels = { send: 'Send message', stop: 'Stop', disabled: 'Send message' };
    btn.setAttribute('aria-label', labels[mode]);
    if (mode === 'stop') btn.dataset.tip = `Stop ${b.name}`;
    else delete btn.dataset.tip;
    composerEl.classList.toggle('disabled', !b);
    layoutComposer();
  }
  input.addEventListener('input', () => {
    updateComposer();
    saveDraftSoon();
  });
  const saveDraftSoon = debounce(() => {
    saveDraft();
    const b = curBot();
    if (b) {
      const c = rowCache.get('row:' + b.id);
      if (c) updateRowEl(c, b);
    }
  }, 500);
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && !e.repeat) {
      e.preventDefault();
      send();
    } else if (e.key === 'Escape') {
      if (S.reply && S.reply.botId === S.current) {
        cancelReply();
        e.stopPropagation();
      } else input.blur();
    }
  });
  input.addEventListener('paste', (e) => {
    const files = Array.from((e.clipboardData && e.clipboardData.files) || []);
    if (files.length) {
      e.preventDefault();
      uploadFiles(files);
    }
  });
  composerEl.addEventListener('submit', (e) => {
    e.preventDefault();
    const mode = $('#sendBtn').dataset.mode;
    if (mode === 'send') send();
    else if (mode === 'stop') stopBot();
  });
  composerEl.addEventListener('mousedown', (e) => {
    if (e.target === composerEl || e.target.id === 'cContent') {
      e.preventDefault();
      input.focus();
    }
  });
  async function stopBot() {
    const b = curBot();
    if (!b) return;
    try {
      await api(`/api/bots/${b.id}/stop`, { method: 'POST' });
    } catch (e) {
      fail(e);
    }
  }
  async function send() {
    const b = curBot();
    if (!b) return;
    if (S.pending.some((p) => p.uploading)) return setNotice('Wait for files to finish uploading');
    if (S.link === 'unreachable' || S.link === 'retrying') return setNotice(`Can’t reach ${computerName()} right now. Your message is still here. Send it once it’s back.`, true);
    if (S.link === 'unpaired') return setNotice(`${ThisDevice()} isn’t paired with ${computerName()} anymore. Pair again to send messages.`, true);
    if (!claudeReady() && (input.value.trim() || S.pending.length)) {
      const c = S.computer.claude;
      setNotice(c.installed ? `Claude isn’t signed in on ${computerName()}. Sign in, then send again.` : `Claude Code isn’t installed on ${computerName()} yet.`, true);
      return openClaudeHelp();
    }
    const text = input.value.trim();
    const atts = S.pending.filter((p) => !p.error && p.path).map(({ name, path, mime, url }) => ({ name, path, mime, url }));
    if (!text && !atts.length) return;
    const pendingCopy = S.pending;
    const reply = S.reply && S.reply.botId === b.id ? S.reply : null;
    S.reply = null;
    renderReplyRow();
    input.value = '';
    S.pending = [];
    S.composerExpanded = false;
    S.drafts.delete(b.id);
    persistDrafts();
    S.newDiv = null;
    renderAttachRow();
    updateComposer();
    const tmp = {
      id: 'tmp-' + Date.now() + Math.random().toString(36).slice(2, 6),
      bot_id: b.id,
      role: 'user',
      text,
      html: '',
      status: 'done',
      meta: Object.assign(
        { attachments: atts.map((a) => Object.assign({}, a, { localUrl: (pendingCopy.find((p) => p.path === a.path) || {}).localUrl })) },
        reply ? { reply_to: { id: reply.id, who: reply.who, quote: reply.quote } } : {}
      ),
      created_at: Date.now(),
      _tmp: true,
    };
    if (!S.msgs.has(b.id)) S.msgs.set(b.id, []);
    S.pinned = true;
    upsertMessage(tmp, true);
    try {
      const m = await api(`/api/bots/${b.id}/messages`, { method: 'POST', body: { text, attachments: atts, reply_to: reply ? reply.id : '' } });
      const list = S.msgs.get(b.id) || [];
      const i = list.findIndex((x) => x.id === tmp.id);
      if (i >= 0) {
        if (m && m.id) adoptRow(tmp.id, m.id);
        list.splice(i, 1);
      }
      if (m && m.id) upsertMessage(m, true);
      else scheduleRender();
    } catch (e) {
      removeMessage(b.id, tmp.id);
      if (S.current === b.id && !input.value) {
        input.value = text;
        S.pending = pendingCopy;
        S.reply = reply;
        renderReplyRow();
        renderAttachRow();
        updateComposer();
      }
      if (e && (e.network || e.status === 401)) {
        noteNetworkTrouble(e.status);
        setNotice(`Couldn’t reach ${computerName()}. Your message is still here.`, true);
      } else setNotice('Couldn’t send your message. ' + (e && e.message ? e.message : ''), true);
    }
  }

  // attachments (max 6 per message, 25 MB each; shell.md 6.6)
  const MAX_FILES = 6;
  const MAX_BYTES = 25 * 1024 * 1024;
  $('#attachBtn').addEventListener('click', () => $('#fileInput').click());
  $('#fileInput').addEventListener('change', (e) => {
    uploadFiles(Array.from(e.target.files || []));
    e.target.value = '';
  });
  function uploadFiles(files) {
    const b = curBot();
    if (!b || !files.length) return;
    let skipped = 0;
    for (const f of files) {
      if (S.pending.length >= MAX_FILES) {
        skipped++;
        continue;
      }
      if (f.size === 0) {
        setNotice(`“${f.name}” is empty, so it wasn’t attached`);
        continue;
      }
      if (f.size > MAX_BYTES) {
        setNotice(`“${f.name}” is too large to attach (max 25 MB)`);
        continue;
      }
      const item = {
        key: Math.random().toString(36).slice(2),
        name: f.name || 'pasted-file',
        mime: f.type,
        size: f.size,
        uploading: true,
        localUrl: f.type && f.type.startsWith('image/') ? URL.createObjectURL(f) : null,
      };
      S.pending.push(item);
      const fd = new FormData();
      fd.append('file', f, item.name);
      api(`/api/bots/${b.id}/upload`, { method: 'POST', form: fd })
        .then((res) => {
          const r = Array.isArray(res) ? res[0] : res;
          Object.assign(item, r || {}, { uploading: false });
        })
        .catch((e) => {
          item.uploading = false;
          item.error = true;
          setNotice(`Couldn’t attach “${item.name}”`, true);
          console.warn(e);
          const arr = S.current === b.id ? S.pending : (S.drafts.get(b.id) || {}).pending || [];
          const i = arr.indexOf(item);
          if (i >= 0) arr.splice(i, 1);
        })
        .finally(() => {
          renderAttachRow();
          updateComposer();
        });
    }
    if (skipped) setNotice(`Only ${MAX_FILES} attachments allowed — ${skipped} wasn’t added`);
    renderAttachRow();
    updateComposer();
    input.focus();
  }
  function renderAttachRow() {
    const row = $('#attachRow');
    row.textContent = '';
    for (const p of S.pending) {
      const remove = h(
        'button',
        {
          class: 'pend-x',
          type: 'button',
          'aria-label': `Remove ${p.name}`,
          onclick: () => {
            S.pending = S.pending.filter((x) => x !== p);
            renderAttachRow();
            updateComposer();
          },
        },
        icon('x', 10, '', 2.6)
      );
      row.append(
        h(
          'div',
          { class: 'pend' + (p.uploading ? ' uploading' : ''), role: 'listitem', title: p.name },
          p.localUrl
            ? h('img', { class: 'pend-thumb', src: p.localUrl, alt: p.name })
            : h('div', { class: 'pend-file' }, h('span', { class: 'fc-icon' }, extOf(p.name).slice(0, 4)), h('span', { class: 'fc-text' }, h('span', { class: 'fc-name' }, p.name), h('span', { class: 'fc-meta' }, p.size ? fmtBytes(p.size) : ''))),
          p.uploading ? h('span', { class: 'pend-spin' }, spinner('xs')) : null,
          remove
        )
      );
    }
    row.hidden = !S.pending.length;
  }

  // drag & drop onto the chat
  let dragDepth = 0;
  const chatEl = $('#chat');
  const hasFiles = (e) => e.dataTransfer && Array.from(e.dataTransfer.types || []).includes('Files');
  chatEl.addEventListener('dragenter', (e) => {
    if (!hasFiles(e) || !curBot()) return;
    e.preventDefault();
    dragDepth++;
    chatEl.classList.add('dragging');
    composerEl.classList.add('dragover');
  });
  chatEl.addEventListener('dragover', (e) => {
    if (hasFiles(e)) e.preventDefault();
  });
  chatEl.addEventListener('dragleave', () => {
    dragDepth = Math.max(0, dragDepth - 1);
    if (!dragDepth) {
      chatEl.classList.remove('dragging');
      composerEl.classList.remove('dragover');
    }
  });
  chatEl.addEventListener('drop', (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    dragDepth = 0;
    chatEl.classList.remove('dragging');
    composerEl.classList.remove('dragover');
    uploadFiles(Array.from(e.dataTransfer.files || []));
  });
  // paste files anywhere (when focus isn't in another field)
  document.addEventListener('paste', (e) => {
    if (isEditable(document.activeElement) || !curBot() || modalStack.length) return;
    const files = Array.from((e.clipboardData && e.clipboardData.files) || []);
    if (files.length) {
      e.preventDefault();
      uploadFiles(files);
    }
  });


  // ───────────────────────────── bot settings panel (header pill, ⌘⇧,) ─────────────────────────────
  const panelEl = $('#panel');
  let P = null; // {botId, els...}
  function setPanel(open, opts = {}) {
    const b = curBot();
    open = !!open && !!b;
    if (S.panelOpen === open && !opts.focusName && !opts.page) return;
    S.panelOpen = open;
    panelEl.hidden = !open;
    layoutShell();
    $('#island').setAttribute('aria-expanded', open ? 'true' : 'false');
    if (open) {
      if (opts.page) S.panelView = { botId: b.id, page: opts.page, tab: 'details' };
      renderPanel();
      if (opts.focusName && P && P.name) setTimeout(() => P.name.focus(), 30);
    } else {
      panelEl.textContent = '';
      P = null;
      sweepAvatars();
    }
  }
  // Bot info pane (settings-and-bots.md 4.6): Details / Library tabs, plus a "Bot settings" sub-page.
  function panelView(b) {
    if (!S.panelView || S.panelView.botId !== b.id) S.panelView = { botId: b.id, page: 'main', tab: 'details' };
    return S.panelView;
  }
  function inlineInput(cls, value, attrs, onCommit, { required = false } = {}) {
    const el = h('input', Object.assign({ class: cls, value: value || '', spellcheck: 'false' }, attrs));
    let start = value || '';
    el.addEventListener('focus', () => (start = el.value));
    el.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && !e.isComposing) el.blur();
      if (e.key === 'Escape') {
        el.value = start;
        el.blur();
        e.stopPropagation();
      }
    });
    el.addEventListener('blur', () => {
      const v = el.value.trim();
      if (required && !v) {
        el.value = start;
        return;
      }
      if (v !== start.trim()) onCommit(v);
    });
    return el;
  }
  function renderPanel() {
    const b = curBot();
    if (!b || !S.panelOpen) return;
    const view = panelView(b);
    panelEl.textContent = '';
    P = { botId: b.id, page: view.page, tab: view.tab };
    const head = h('div', { class: 'panel-head' });
    if (isGroup(b)) {
      head.append(h('span', { class: 'panel-head-title strong' }, 'Group chat'));
      head.append(h('button', { class: 'ghost-btn', 'aria-label': 'Close', 'data-tip': 'Close', 'data-kbd': 'Esc', onclick: () => setPanel(false) }, icon('x', 16)));
      const body = h('div', { class: 'panel-body' });
      panelEl.append(head, body);
      panelGroup(b, body);
      renderChatHeader();
      return;
    }
    if (view.page === 'settings') {
      head.append(
        h(
          'div',
          { class: 'panel-head-start' },
          h(
            'button',
            {
              class: 'ghost-btn',
              'aria-label': 'Back',
              'data-tip': 'Back',
              onclick: () => {
                view.page = 'main';
                renderPanel();
              },
            },
            icon('chevronLeft', 16)
          ),
          h('span', { class: 'panel-head-title strong' }, 'Bot settings')
        )
      );
    } else {
      head.append(
        h(
          'button',
          {
            class: 'ghost-btn',
            'aria-label': 'Bot settings',
            'data-tip': 'Bot settings',
            onclick: () => {
              view.page = 'settings';
              renderPanel();
            },
          },
          icon('gear', 16)
        )
      );
    }
    head.append(h('button', { class: 'ghost-btn', 'aria-label': 'Close', 'data-tip': 'Close', 'data-kbd': 'Esc', onclick: () => setPanel(false) }, icon('x', 16)));
    const body = h('div', { class: 'panel-body' });
    panelEl.append(head, body);
    if (view.page === 'settings') panelSettingsPage(b, body);
    else {
      const tabs = h(
        'div',
        { class: 'p-tabs', role: 'tablist' },
        [
          ['details', 'Details'],
          ['library', 'Library'],
        ].map(([k, l]) =>
          h(
            'button',
            {
              class: 'p-tab' + (view.tab === k ? ' on' : ''),
              role: 'tab',
              'aria-selected': view.tab === k ? 'true' : 'false',
              onclick: () => {
                if (view.tab === k) return;
                view.tab = k;
                renderPanel();
              },
            },
            l
          )
        )
      );
      body.append(tabs);
      if (view.tab === 'library') panelLibrary(b, body);
      else panelDetails(b, body);
    }
    renderChatHeader();
  }
  function panelDetails(b, body) {
    const av = avatar(b, 72, { live: true, state: botState(b), interactive: true, badge: b.main ? 'main' : null });
    av.addEventListener('click', () => pokeAvatar(av));
    av.setAttribute('aria-label', 'Poke ' + b.name);
    const name = inlineInput('p-name', b.name, { maxlength: '255', 'aria-label': 'Bot name' }, (v) => patchBot(b.id, { name: v }), { required: true });
    const label = inlineInput('p-label-in', b.label, { maxlength: '80', placeholder: 'Add a label', 'aria-label': 'Label' }, (v) => patchBot(b.id, { label: v }));
    const about = h('textarea', { class: 'p-text', placeholder: 'Tell this Bot what to do. It follows these instructions in every conversation.', 'aria-label': 'Instructions' });
    about.value = b.instructions || '';
    const more = h('button', { class: 'p-more', hidden: true }, 'Show more');
    let expanded = false;
    const growAbout = () => {
      about.style.height = 'auto';
      const full = about.scrollHeight;
      const cap = expanded || document.activeElement === about ? 480 : 132;
      about.style.height = clamp(full, 64, cap) + 'px';
      more.hidden = full <= 132;
      more.textContent = expanded ? 'Show less' : 'Show more';
    };
    more.addEventListener('click', () => {
      expanded = !expanded;
      growAbout();
    });
    about.addEventListener('input', growAbout);
    about.addEventListener('focus', growAbout);
    about.addEventListener('blur', () => {
      growAbout();
      const cur = S.bots.get(b.id);
      if (cur && about.value.trim() !== (cur.instructions || '').trim()) patchBot(b.id, { instructions: about.value.trim() }).then((nb) => nb && toast('Instructions saved'));
    });
    const routines = h('div', { class: 'panel-card' }, h('div', { class: 'p-empty' }, spinner('xs')));
    const skills = h('div', { class: 'panel-card' }, h('div', { class: 'p-empty' }, spinner('xs')));
    const memories = h('div', { class: 'panel-card' }, h('div', { class: 'p-empty' }, spinner('xs')));
    const addSkill = h('button', { class: 'ghost-btn sm', 'aria-label': 'Add skill', 'data-tip': 'Add skill', onclick: () => openSkillEditor(S.bots.get(b.id), null) }, icon('plus', 14));
    body.append(
      h(
        'div',
        { class: 'p-hero' },
        h('div', { class: 'p-avatar' }, av),
        name,
        h('div', { class: 'p-sub' }, b.running ? (b.activity && b.activity.label) || 'Working' : 'Only you can see it'),
        label
      ),
      h('div', { class: 'p-sec' }, h('div', { class: 'p-label' }, 'Instructions'), h('div', { class: 'panel-card' }, about), more),
      h('div', { class: 'p-sec' }, h('div', { class: 'p-label' }, h('span', null, 'Routines')), routines),
      h('div', { class: 'p-sec' }, h('div', { class: 'p-label' }, h('span', null, 'Skills'), addSkill), skills),
      h('div', { class: 'p-sec' }, h('div', { class: 'p-label' }, h('span', null, 'Memories')), memories)
    );
    Object.assign(P, { av, name, label, about, routines, skills, memories });
    requestAnimationFrame(growAbout);
    loadPanelLists();
  }
  function panelSettingsPage(b, body) {
    const avBtn = h(
      'button',
      { class: 'p-av-btn', 'aria-label': 'Edit Bot avatar', 'data-tip': 'Edit Bot avatar', onclick: () => openAvatarEditor(S.bots.get(b.id)) },
      avatar(b, 56, { badge: b.main ? 'main' : null }),
      h('span', { class: 'p-avatar-edit' }, icon('pencil', 12, '', 2))
    );
    const name = inlineInput('field', b.name, { maxlength: '255', placeholder: 'Bob', 'aria-label': 'Name' }, (v) => patchBot(b.id, { name: v }), { required: true });
    const label = inlineInput('field', b.label, { maxlength: '80', placeholder: 'Research, marketing, admin', 'aria-label': 'Label (optional)' }, (v) => patchBot(b.id, { label: v }));
    const notifSub = h('div', { class: 'p-row-sub' }, 'Get notified when this Bot finishes or is waiting for you');
    const notif = toggleEl(b.notifications !== false, async (v) => {
      const nb = await patchBotQuiet(b.id, { notifications: v });
      if (!nb) {
        notif.checked = !v;
        notifSub.textContent = 'Couldn’t save. Try again.';
        notifSub.classList.add('err');
      } else {
        notifSub.textContent = 'Get notified when this Bot finishes or is waiting for you';
        notifSub.classList.remove('err');
      }
    });
    notif.classList.add('sm');
    notif.setAttribute('aria-label', 'Notifications');
    const model = selectEl(b.model || '', modelOptions('Same as Settings', b.model), (v) => patchBot(b.id, { model: v }));
    model.classList.add('p-select');
    const local = S.conn.mode !== 'remote';
    const hideBtn = h('button', { class: 'p-action', onclick: () => setHidden(S.bots.get(b.id), !S.bots.get(b.id).hidden) }, icon(b.hidden ? 'eye' : 'eyeOff', 16), b.hidden ? 'Show in sidebar' : 'Hide from sidebar');
    body.append(
      h('div', { class: 'p-hero left' }, avBtn),
      h('label', { class: 'p-flabel' }, 'Name'),
      name,
      h('label', { class: 'p-flabel' }, 'Label (optional)'),
      label,
      h('div', { class: 'p-sec' }, h('div', { class: 'panel-card' }, h('div', { class: 'p-row' }, h('div', { class: 'p-row-text' }, h('div', { class: 'p-row-title' }, 'Notifications'), notifSub), notif))),
      h('div', { class: 'p-sec' }, h('div', { class: 'p-label' }, 'Model'), model),
      b.workdir &&
        h(
          'div',
          { class: 'p-sec' },
          h('div', { class: 'p-label' }, 'Folder'),
          h(
            'div',
            { class: 'panel-card' },
            h(
              'div',
              { class: 'p-row' },
              icon('folder', 16),
              h('div', { class: 'p-row-text' }, h('div', { class: 'p-row-sub mono', title: b.workdir, style: 'white-space:nowrap;overflow:hidden;text-overflow:ellipsis;direction:rtl;text-align:left' }, b.workdir)),
              local && h('button', { class: 'btn small', onclick: () => api('/api/open-folder', { method: 'POST', body: { path: b.workdir } }).catch(fail) }, 'Open')
            )
          )
        ),
      h(
        'div',
        { class: 'p-actions' },
        hideBtn,
        h('button', { class: 'p-action', onclick: () => resetBot(S.bots.get(b.id)) }, icon('reset', 16), 'Reset conversation'),
        b.main
          ? h('button', { class: 'p-action', onclick: () => choosePrimaryBot(S.bots.get(b.id)) }, icon('refresh', 16), 'Replace with different Bot')
          : h('button', { class: 'p-action danger', onclick: () => deleteBot(S.bots.get(b.id)) }, icon('trash', 16), 'Delete Bot')
      )
    );
    Object.assign(P, { nameField: name, labelField: label, notif, model });
  }
  async function patchBotQuiet(id, body) {
    try {
      const nb = await api(`/api/bots/${id}`, { method: 'PATCH', body });
      if (nb && nb.id) S.bots.set(nb.id, Object.assign(S.bots.get(nb.id) || {}, nb));
      return nb;
    } catch {
      return null;
    }
  }
  function panelLibrary(b, body) {
    const box = h('div', { class: 'p-lib' }, h('div', { class: 'p-empty' }, spinner('xs')));
    body.append(box);
    P.library = box;
    api(`/api/bots/${b.id}/library`)
      .then((list) => {
        if (!P || P.botId !== b.id || P.library !== box) return;
        box.textContent = '';
        list = Array.isArray(list) ? list : [];
        if (!list.length) return box.append(h('div', { class: 'p-empty center' }, `No files yet. Files ${b.name} makes or you send it show up here.`));
        for (const f of list) {
          const img = isImage(f) && !/\.(heic|svg)$/i.test(f.name || '');
          box.append(
            h(
              'a',
              { class: 'lib-row', href: fileUrl(f.url), target: '_blank', rel: 'noopener', title: f.path || f.name },
              img ? h('img', { class: 'lib-thumb', src: fileUrl(f.url), alt: '', loading: 'lazy' }) : h('span', { class: 'lib-icon' }, h('span', null, extOf(f.name).slice(0, 4))),
              h('span', { class: 'p-row-text' }, h('span', { class: 'p-row-title ellipsis' }, f.name), h('span', { class: 'p-row-sub' }, [fmtBytes(f.size || 0), f.modified ? timeLabel(f.modified) : ''].filter(Boolean).join(' · ')))
            )
          );
        }
      })
      .catch(() => {
        if (P && P.library === box) box.replaceChildren(h('div', { class: 'p-empty center' }, 'Couldn’t load files.'));
      });
  }
  function refreshPanelBot() {
    const b = curBot();
    if (!P || !b) return;
    if (P.botId !== b.id) return renderPanel();
    if (isGroup(b)) {
      if (!(document.activeElement && panelEl.contains(document.activeElement))) renderPanel();
      return;
    }
    const ae = document.activeElement;
    if (P.av) syncAvatar(P.av, b, 72, { live: true, state: botState(b), badge: b.main ? 'main' : null });
    if (P.name && ae !== P.name) P.name.value = b.name;
    if (P.label && ae !== P.label) P.label.value = b.label || '';
    if (P.about && ae !== P.about) P.about.value = b.instructions || '';
    if (P.nameField && ae !== P.nameField) P.nameField.value = b.name;
    if (P.labelField && ae !== P.labelField) P.labelField.value = b.label || '';
    if (P.notif) P.notif.checked = b.notifications !== false;
    if (P.model && ae !== P.model) P.model.value = b.model || '';
    if (P.page === 'settings' && !(ae && panelEl.contains(ae))) renderPanel();
  }
  async function loadPanelLists(which) {
    if (!P || !P.routines) return;
    const id = P.botId;
    const still = () => P && P.botId === id && P.routines;
    if (!which || which === 'routines') {
      api(`/api/bots/${id}/routines`)
        .then((r) => still() && paintRoutines(Array.isArray(r) ? r : []))
        .catch(() => still() && paintRoutines(null));
    }
    if (!which || which === 'memories') {
      api(`/api/bots/${id}/memories`)
        .then((r) => still() && paintMemories(Array.isArray(r) ? r : []))
        .catch(() => still() && paintMemories(null));
    }
    if (!which || which === 'skills') {
      api(`/api/bots/${id}/skills`)
        .then((r) => still() && paintSkills(Array.isArray(r) ? r : []))
        .catch(() => still() && paintSkills(null));
    }
  }

  // ── routines (settings-and-bots.md 5.2 / 5.3) ──
  const routineRuns = new Map(); // routine id → started ms
  const routineRunning = (r) => {
    const t = routineRuns.get(r.id);
    if (!t) return false;
    const b = S.bots.get(r.bot_id);
    if (Date.now() - t < 4000 || (b && b.running && Date.now() - t < 15 * 60000)) return true;
    routineRuns.delete(r.id);
    return false;
  };
  async function setRoutineEnabled(r, on) {
    try {
      await api(`/api/routines/${encodeURIComponent(r.id)}`, { method: 'PATCH', body: { enabled: on } });
      r.enabled = on;
      return true;
    } catch {
      toast(on ? 'Couldn’t resume this routine' : 'Couldn’t pause this routine', 'error');
      return false;
    } finally {
      loadPanelLists('routines');
    }
  }
  async function testRoutine(r) {
    routineRuns.set(r.id, Date.now());
    try {
      await api(`/api/routines/${encodeURIComponent(r.id)}/run`, { method: 'POST' });
    } catch (e) {
      routineRuns.delete(r.id);
      toast('Couldn’t start a test run. ' + (e && e.network ? 'Check your connection and try again' : 'This Bot is unavailable'), 'error');
    }
  }
  async function deleteRoutine(r) {
    const ok = await confirmDialog({ title: `Delete “${r.name || 'Routine'}”`, body: 'This deletes the routine and stops its future runs. This can\'t be undone.', confirmText: 'Delete', danger: true });
    if (!ok) return false;
    try {
      await api(`/api/routines/${encodeURIComponent(r.id)}`, { method: 'DELETE' });
      loadPanelLists('routines');
      return true;
    } catch (e) {
      fail(e);
      return false;
    }
  }
  function routineMenu(r, anchor) {
    const running = routineRunning(r);
    menuAt(anchor, [
      { label: r.enabled ? 'Pause' : 'Resume', icon: r.enabled ? 'pause' : 'play', action: () => setRoutineEnabled(r, !r.enabled) },
      { label: running ? 'Running…' : 'Test', icon: 'bolt', disabled: running, action: () => testRoutine(r) },
      { label: 'Edit', icon: 'pencil', action: () => openRoutine(r, true) },
      '-',
      { label: 'Delete', icon: 'trash', danger: true, action: () => deleteRoutine(r) },
    ]);
  }
  function paintRoutines(list) {
    const box = P.routines;
    box.textContent = '';
    const b = curBot();
    if (list == null) return box.append(h('div', { class: 'p-empty' }, 'Couldn’t load routines.'));
    if (!list.length) return box.append(h('div', { class: 'p-empty' }, `No routines yet. Ask ${b ? b.name : 'this Bot'} to do something on a schedule, like “every weekday at 8, summarize my inbox.”`));
    for (const r of list) {
      const sched = r.schedule_text || 'On a schedule';
      const sw = toggleEl(!!r.enabled, async (v) => {
        if (!(await setRoutineEnabled(r, v))) sw.checked = !v;
      });
      sw.classList.add('sm');
      sw.setAttribute('aria-label', `${r.enabled ? 'Pause' : 'Resume'} ${r.name || 'routine'}, ${sched}`);
      sw.addEventListener('click', (e) => e.stopPropagation());
      const rowEl = h(
        'div',
        {
          class: 'p-row clickable',
          role: 'button',
          tabindex: '0',
          'aria-haspopup': 'menu',
          'aria-label': `Routine actions, ${r.name || 'Routine'}`,
          onclick: () => routineMenu(r, rowEl),
          onkeydown: (e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              routineMenu(r, rowEl);
            }
          },
        },
        h('div', { class: 'p-row-text' }, h('div', { class: 'p-row-title' }, r.name || r.prompt || 'Routine'), h('div', { class: 'p-row-sub' }, r.enabled ? (r.paused_for_unread ? `${sched} · Paused until you read new messages` : sched) : 'Paused')),
        sw
      );
      box.append(rowEl);
    }
  }
  // Routine detail (5.3): Instruction, When to run, footer Pause/Resume · Test · Edit · Delete
  function openRoutine(r, editing = false) {
    const box = h('div', { class: 'dialog routine-dialog', role: 'dialog', 'aria-modal': 'true', 'aria-label': r.name || 'Routine' });
    const close = openModal(box);
    const paint = () => {
      box.textContent = '';
      const nameIn = h('input', { class: 'field', value: r.name || '', maxlength: '120', 'aria-label': 'Name' });
      const promptIn = h('textarea', { class: 'field', rows: '6', 'aria-label': 'Instruction' });
      promptIn.value = r.prompt || '';
      const running = routineRunning(r);
      box.append(
        h('button', { class: 'icon-btn dialog-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 16)),
        editing ? h('div', { style: 'margin-right:36px' }, nameIn) : h('h2', { class: 'dialog-title' }, r.name || 'Routine'),
        h('div', { class: 'rt-label' }, 'Instruction'),
        editing ? promptIn : h('div', { class: 'rt-text' }, r.prompt || ''),
        h('div', { class: 'rt-label' }, 'When to run'),
        h('div', { class: 'rt-when' }, icon('clock', 15), h('span', null, r.schedule_text || 'On a schedule'), !r.enabled && h('span', { class: 'chip' }, 'Paused')),
        editing
          ? h(
              'div',
              { class: 'dialog-actions' },
              h(
                'button',
                {
                  class: 'btn',
                  onclick: () => {
                    editing = false;
                    paint();
                  },
                },
                'Cancel'
              ),
              h(
                'button',
                {
                  class: 'btn primary',
                  onclick: async (e) => {
                    const name = nameIn.value.trim();
                    const prompt = promptIn.value.trim();
                    if (!name || !prompt) return (name ? promptIn : nameIn).focus();
                    e.currentTarget.disabled = true;
                    try {
                      const nr = await api(`/api/routines/${encodeURIComponent(r.id)}`, { method: 'PATCH', body: { name, prompt } });
                      Object.assign(r, nr && nr.id ? nr : { name, prompt });
                      editing = false;
                      paint();
                      loadPanelLists('routines');
                    } catch (err) {
                      e.currentTarget.disabled = false;
                      fail(err);
                    }
                  },
                },
                'Save'
              )
            )
          : h(
              'div',
              { class: 'dialog-actions rt-foot' },
              h('button', { class: 'btn', onclick: async () => (await setRoutineEnabled(r, !r.enabled)) && paint() }, r.enabled ? 'Pause' : 'Resume'),
              h(
                'button',
                {
                  class: 'btn',
                  disabled: running,
                  onclick: async () => {
                    await testRoutine(r);
                    paint();
                  },
                },
                running ? 'Running…' : 'Test'
              ),
              h(
                'button',
                {
                  class: 'btn',
                  onclick: () => {
                    editing = true;
                    paint();
                  },
                },
                'Edit'
              ),
              h('span', { style: 'flex:1' }),
              h('button', { class: 'circle-btn danger', 'aria-label': 'Delete routine', 'data-tip': 'Delete routine', onclick: async () => (await deleteRoutine(r)) && close() }, icon('trash', 16))
            )
      );
      if (editing) setTimeout(() => promptIn.focus(), 30);
    };
    paint();
  }

  // ── skills (Claude Code skills in the bot's folder) ──
  function paintSkills(list) {
    const box = P.skills;
    box.textContent = '';
    const b = curBot();
    if (list == null) return box.append(h('div', { class: 'p-empty' }, 'Couldn’t load skills.'));
    if (!list.length) return box.append(h('div', { class: 'p-empty' }, 'No skills yet. A skill is a saved set of steps the Bot can follow, like “weekly-report”.'));
    for (const sk of list) {
      box.append(
        h(
          'div',
          { class: 'p-row clickable', role: 'button', tabindex: '0', onclick: () => openSkillEditor(b, sk), onkeydown: (e) => e.key === 'Enter' && openSkillEditor(b, sk) },
          icon('bolt', 15),
          h('div', { class: 'p-row-text' }, h('div', { class: 'p-row-title mono' }, sk.name), sk.description && h('div', { class: 'p-row-sub' }, sk.description)),
          icon('chevronRight', 14)
        )
      );
    }
  }
  function openSkillEditor(b, sk) {
    if (!b) return;
    const nameIn = h('input', { class: 'field mono', value: sk ? sk.name : '', placeholder: 'weekly-report', maxlength: '64', spellcheck: 'false', 'aria-label': 'Name', disabled: !!sk });
    const descIn = h('input', { class: 'field', value: sk ? sk.description || '' : '', placeholder: 'When should the Bot use this skill?', maxlength: '300', 'aria-label': 'Description' });
    const bodyIn = h('textarea', { class: 'field', rows: '9', placeholder: 'The steps the Bot follows', 'aria-label': 'Instructions' });
    bodyIn.value = sk ? sk.body || '' : '';
    const err = h('div', { class: 'dialog-error', role: 'alert', hidden: true });
    nameIn.addEventListener('input', () => {
      const p = nameIn.selectionStart;
      nameIn.value = nameIn.value.toLowerCase().replace(/[\s_]+/g, '-').replace(/[^a-z0-9-]/g, '');
      nameIn.setSelectionRange(p, p);
    });
    const save = async (e) => {
      const name = nameIn.value.trim().replace(/^-+|-+$/g, '');
      err.hidden = true;
      const bad = (msg, el) => {
        err.textContent = msg;
        err.hidden = false;
        el.focus();
      };
      if (!/^[a-z0-9]+(-[a-z0-9]+)*$/.test(name)) return bad('Use lowercase letters, numbers and dashes, like weekly-report', nameIn);
      if (!bodyIn.value.trim()) return bad('Write the skill’s instructions', bodyIn);
      e.currentTarget.disabled = true;
      try {
        await api(`/api/bots/${b.id}/skills/${encodeURIComponent(name)}`, { method: 'PUT', body: { description: descIn.value.trim(), body: bodyIn.value } });
        close();
        loadPanelLists('skills');
      } catch (x) {
        e.currentTarget.disabled = false;
        bad(x.message, nameIn);
      }
    };
    const box = h(
      'div',
      { class: 'dialog', role: 'dialog', 'aria-modal': 'true', 'aria-label': sk ? 'Edit skill' : 'New skill' },
      h('button', { class: 'icon-btn dialog-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 16)),
      h('h2', { class: 'dialog-title' }, sk ? 'Edit skill' : 'New skill'),
      h('label', { class: 'f-label' }, 'Name'),
      nameIn,
      h('label', { class: 'f-label' }, 'Description'),
      descIn,
      h('label', { class: 'f-label' }, 'Instructions'),
      bodyIn,
      err,
      h(
        'div',
        { class: 'dialog-actions' },
        sk &&
          h(
            'button',
            {
              class: 'btn danger',
              onclick: async () => {
                const ok = await confirmDialog({ title: `Delete “${sk.name}”`, body: 'The Bot can no longer use this skill. This can\'t be undone.', confirmText: 'Delete', danger: true });
                if (!ok) return;
                try {
                  await api(`/api/bots/${b.id}/skills/${encodeURIComponent(sk.name)}`, { method: 'DELETE' });
                  close();
                  loadPanelLists('skills');
                } catch (x) {
                  fail(x);
                }
              },
            },
            'Delete'
          ),
        h('span', { style: 'flex:1' }),
        h('button', { class: 'btn', onclick: () => close() }, 'Cancel'),
        h('button', { class: 'btn primary', onclick: save }, sk ? 'Save' : 'Add skill')
      )
    );
    const close = openModal(box);
    setTimeout(() => (sk ? bodyIn : nameIn).focus(), 40);
  }

  // The panel shows one summary row; the list itself opens in a window (openMemories), so a Bot
  // with dozens of memories doesn't turn the panel into a wall of cards, and facts shared by all
  // Bots aren't repeated in every Bot's panel.
  function paintMemories(list) {
    const box = P.memories;
    box.textContent = '';
    if (list == null) return box.append(h('div', { class: 'p-empty' }, 'Couldn’t load memories.'));
    if (!list.length) return box.append(h('div', { class: 'p-empty' }, 'Nothing remembered yet. Say “remember that…” and it shows up here.'));
    const shared = list.filter((mm) => !mm.bot_id).length;
    const own = list.length - shared;
    const sub = [own && `${own} for this Bot`, shared && `${shared} about you`].filter(Boolean).join(' · ');
    const id = P.botId;
    const open = () => openMemories(S.bots.get(id));
    box.append(
      h(
        'div',
        {
          class: 'p-row clickable',
          role: 'button',
          tabindex: '0',
          'aria-label': 'See memories',
          onclick: open,
          onkeydown: (e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              open();
            }
          },
        },
        h('div', { class: 'p-row-text' }, h('div', { class: 'p-row-title' }, list.length === 1 ? '1 memory' : `${list.length} memories`), h('div', { class: 'p-row-sub' }, sub)),
        icon('chevronRight', 14)
      )
    );
  }
  // "[2026-11-02] - text" → { date: "Nov 2", text }
  function memoryParts(t) {
    const m = /^\s*\[(\d{4})-(\d{2})-(\d{2})\]\s*(?:-\s*)?/.exec(t || '');
    if (!m) return { date: '', text: t || '' };
    const d = new Date(+m[1], +m[2] - 1, +m[3]);
    const date = d.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: d.getFullYear() === new Date().getFullYear() ? undefined : 'numeric' });
    return { date, text: t.slice(m[0].length) };
  }
  function openMemories(b) {
    if (!b) return;
    const q = h('input', { class: 'search-input', type: 'search', placeholder: 'Search memories', 'aria-label': 'Search memories' });
    const list = h('div', { class: 'mem-list' }, h('div', { class: 'p-empty' }, spinner('xs')));
    let all = [];
    const paint = () => {
      list.textContent = '';
      const words = q.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
      const hit = (mm) => words.every((w) => (mm.text || '').toLowerCase().includes(w));
      const shown = all.filter(hit);
      const groups = [
        ['About you · shared with all Bots', shown.filter((mm) => !mm.bot_id && mm.tier !== 'log')],
        [`${b.name} · always remembered`, shown.filter((mm) => mm.bot_id && mm.tier !== 'log')],
        ['Notes · looked up when needed', shown.filter((mm) => mm.tier === 'log').reverse()],
      ].filter(([, l]) => l.length);
      if (!groups.length) return list.append(h('div', { class: 'p-empty center' }, all.length ? 'No memories match.' : 'Nothing remembered yet.'));
      for (const [label, items] of groups) {
        list.append(h('div', { class: 'p-group' }, `${label} (${items.length})`));
        const card = h('div', { class: 'panel-card' });
        for (const mm of items) card.append(memRow(mm));
        list.append(card);
      }
    };
    const memRow = (mm) => {
      const { date, text } = memoryParts(mm.text);
      const body = h('div', { class: 'mem-text', title: 'Show all' }, text);
      body.addEventListener('click', () => body.classList.toggle('open'));
      const row = h(
        'div',
        { class: 'p-row mem-row' },
        h('div', { class: 'p-row-text' }, body, date && h('div', { class: 'p-row-sub' }, date)),
        h(
          'button',
          {
            class: 'ghost-btn sm danger mem-del',
            'aria-label': 'Forget this',
            'data-tip': mm.bot_id ? 'Forget' : 'Forget (for all Bots)',
            onclick: async () => {
              try {
                await api(`/api/memories/${encodeURIComponent(mm.id)}`, { method: 'DELETE' });
                all = all.filter((x) => x.id !== mm.id);
                paint();
                loadPanelLists('memories');
              } catch (e) {
                fail(e);
              }
            },
          },
          icon('trash', 14)
        )
      );
      return row;
    };
    q.addEventListener('input', paint);
    const box = h(
      'div',
      { class: 'dialog wide mem-dlg', role: 'dialog', 'aria-modal': 'true', 'aria-label': `${b.name}’s memories` },
      h('button', { class: 'icon-btn dialog-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 16)),
      h('h2', { class: 'dialog-title' }, 'Memories'),
      h('p', { class: 'dialog-body' }, `What ${b.name} remembers. Facts about you are shared with all your Bots.`),
      h('div', { class: 'mem-search' }, icon('search', 15), q),
      list
    );
    const close = openModal(box);
    api(`/api/bots/${b.id}/memories`)
      .then((r) => {
        all = Array.isArray(r) ? r : [];
        paint();
      })
      .catch(() => {
        list.textContent = '';
        list.append(h('div', { class: 'p-empty' }, 'Couldn’t load memories.'));
      });
    setTimeout(() => q.focus(), 40);
  }

  // ───────────────────────────── avatar picker (skin grids) ─────────────────────────────
  // st = {shape, color}; onChange(next) is called on every pick. grid = solid colours, sw = patterns.
  function avatarPicker(st, onChange) {
    const grid = h('div', { class: 'shape-grid', role: 'radiogroup', 'aria-label': 'Character color' });
    const sw = h('div', { class: 'shape-grid', role: 'radiogroup', 'aria-label': 'Character pattern' });
    function paint() {
      grid.textContent = '';
      sw.textContent = '';
      for (const skin of skinList()) {
        const on = st.color === skin.id;
        const cell = h(
          'button',
          {
            class: 'shape-cell' + (on ? ' on' : ''),
            role: 'radio',
            'aria-checked': on ? 'true' : 'false',
            'aria-label': skin.label,
            'data-tip': skin.label,
            onclick: () => {
              st.shape = 'ball';
              st.color = skin.id;
              paint();
              onChange(Object.assign({}, st));
            },
          },
          avatar({ shape: 'ball', color: skin.id }, 36)
        );
        (skin.kind === 'pattern' ? sw : grid).append(cell);
      }
    }
    paint();
    return { grid, sw, paint };
  }
  function openAvatarEditor(b) {
    if (!b) return;
    const st = lookOf(b);
    const prev = h('span', { class: 'av', style: { width: '96px', height: '96px' } });
    prev._size = 96;
    paintAvatar(prev, st, 96, { live: true, interactive: true });
    prev.addEventListener('click', () => pokeAvatar(prev));
    const save = debounce((next) => patchBot(b.id, { shape: next.shape, color: next.color }), 250);
    const picker = avatarPicker(st, (next) => {
      paintAvatar(prev, next, 96, { live: true, interactive: true });
      pokeAvatar(prev);
      save(next);
      resetLink.hidden = false;
    });
    const resetLink = h(
      'button',
      {
        class: 'link-btn',
        hidden: !(b.shape || b.color),
        onclick: async () => {
          const nb = await patchBot(b.id, { shape: '', color: '' });
          if (nb) {
            const d = lookOf(nb);
            st.shape = d.shape;
            st.color = d.color;
            picker.paint();
            paintAvatar(prev, d, 96, { live: true, interactive: true });
            resetLink.hidden = true;
          }
        },
      },
      'Reset character to default'
    );
    const box = h(
      'div',
      { class: 'dialog', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Avatar editor' },
      h('button', { class: 'icon-btn dialog-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 16)),
      h('h2', { class: 'dialog-title' }, b.name),
      h('div', { class: 'ne-preview' }, prev),
      h('label', { class: 'f-label' }, 'Color'),
      picker.grid,
      h('label', { class: 'f-label' }, 'Pattern'),
      picker.sw,
      h('div', { class: 'dialog-actions', style: 'justify-content:space-between;align-items:center' }, resetLink, h('span', { style: 'flex:1' }), h('button', { class: 'btn primary', onclick: () => close() }, 'Done'))
    );
    const close = openModal(box);
  }

  // ───────────────────────────── "Create New Bot" (settings-and-bots.md 4.2) ─────────────────────────────
  // ── group chats ──
  const MAX_GROUP = 6;
  const memberNames = (g) => {
    const names = (g.members || []).map((id) => S.bots.get(id)).filter(Boolean).map((x) => x.name);
    if (names.length <= 2) return names.join(' and ') || 'No Bots yet';
    return names.slice(0, -1).join(', ') + ', and ' + names[names.length - 1];
  };
  const GROUP_COST_NOTE = 'Group chats use more of your Claude usage, because each Bot reads every message. For most work, chat with one Bot instead.';
  function panelGroup(g, body) {
    const av = avatar(g, 72);
    const name = inlineInput('p-name', g.name, { maxlength: '72', 'aria-label': 'Group name', placeholder: memberNames(g) }, (v) => patchBot(g.id, { name: v }));
    const list = h('div', { class: 'panel-card g-members' });
    const members = (g.members || []).map((id) => S.bots.get(id)).filter(Boolean);
    for (const m of members) {
      list.append(
        h(
          'div',
          { class: 'g-member' },
          avatar(m, 28),
          h('button', { class: 'g-member-name', onclick: () => selectBot(m.id) }, m.name),
          members.length > 1
            ? h('button', { class: 'ghost-btn sm', 'aria-label': `Remove ${m.name}`, 'data-tip': `Remove ${m.name}`, onclick: () => patchBot(g.id, { members: g.members.filter((x) => x !== m.id) }) }, icon('x', 14))
            : null
        )
      );
    }
    const canAdd = members.length < MAX_GROUP && realBots().some((x) => !g.members.includes(x.id));
    const addBtn = h('button', { class: 'ghost-btn sm', 'aria-label': 'Add Bots', 'data-tip': canAdd ? 'Add Bots' : `Up to ${MAX_GROUP} Bots`, disabled: !canAdd, onclick: () => openGroupPicker(g) }, icon('plus', 14));
    const notif = toggleEl(g.notifications !== false, (on) => patchBot(g.id, { notifications: on }));
    body.append(
      h('div', { class: 'p-hero' }, h('div', { class: 'p-avatar' }, av), name, h('div', { class: 'p-sub' }, g.running ? (g.activity && g.activity.label) || 'Working' : `${members.length} ${members.length === 1 ? 'Bot' : 'Bots'}`)),
      h('div', { class: 'p-sec' }, h('div', { class: 'p-label' }, h('span', null, 'Bots'), addBtn), list),
      h('div', { class: 'p-sec' }, h('div', { class: 'panel-card g-row' }, h('span', null, 'Notifications'), notif)),
      h('div', { class: 'p-sec' }, h('div', { class: 'p-note' }, GROUP_COST_NOTE)),
      h('div', { class: 'p-sec' }, h('button', { class: 'btn danger', onclick: () => deleteBot(S.bots.get(g.id)) }, 'Delete group chat'))
    );
    Object.assign(P, { name });
  }
  // Pick Bots: for a new group (g = null) or to add to one.
  function botChecklist(chosen, onChange, exclude = []) {
    const box = h('div', { class: 'g-pick', role: 'group', 'aria-label': 'Bots' });
    const bots = orderedBots().filter((b) => !isGroup(b) && !exclude.includes(b.id));
    const paint = () => {
      box.textContent = '';
      for (const b of bots) {
        const on = chosen.includes(b.id);
        const full = !on && chosen.length + exclude.length >= MAX_GROUP;
        box.append(
          h(
            'button',
            {
              class: 'g-pick-row' + (on ? ' on' : ''),
              role: 'checkbox',
              'aria-checked': on ? 'true' : 'false',
              disabled: full,
              onclick: () => {
                if (on) chosen.splice(chosen.indexOf(b.id), 1);
                else chosen.push(b.id);
                paint();
                onChange();
              },
            },
            avatar(b, 28),
            h('span', { class: 'g-pick-name' }, b.name, b.label ? h('span', { class: 'g-pick-sub' }, b.label) : null),
            h('span', { class: 'g-check' }, on ? icon('check', 13, '', 2.4) : null)
          )
        );
      }
    };
    paint();
    return box;
  }
  function openGroupPicker(g) {
    const chosen = [];
    const addBtn = h('button', { class: 'btn primary', disabled: true }, 'Add');
    const list = botChecklist(chosen, () => (addBtn.disabled = !chosen.length), g.members);
    addBtn.addEventListener('click', async () => {
      close();
      await patchBot(g.id, { members: [...g.members, ...chosen] });
    });
    const box = h(
      'div',
      { class: 'dialog', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Add Bots' },
      h('button', { class: 'icon-btn dialog-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 16)),
      h('h2', { class: 'dialog-title' }, 'Add to group chat'),
      list,
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn', onclick: () => close() }, 'Cancel'), addBtn)
    );
    const close = openModal(box);
  }
  function openNewGroup() {
    const chosen = [];
    const nameIn = h('input', { class: 'field', placeholder: 'Group name (optional)', maxlength: '72', 'aria-label': 'Group name' });
    const err = h('div', { class: 'dialog-error', role: 'alert', hidden: true });
    const createBtn = h('button', { class: 'btn primary', disabled: true }, 'Create group chat');
    const sync = () => {
      createBtn.disabled = busy || chosen.length < 2;
      nameIn.placeholder = chosen.length ? memberNames({ members: chosen }) : 'Group name (optional)';
    };
    let busy = false;
    const list = botChecklist(chosen, sync);
    createBtn.addEventListener('click', async () => {
      if (busy || chosen.length < 2) return;
      busy = true;
      sync();
      err.hidden = true;
      try {
        const g = await api('/api/groups', { method: 'POST', body: { name: nameIn.value.trim(), members: chosen } });
        close();
        if (g && g.id) {
          S.bots.set(g.id, g);
          renderSidebar();
          selectBot(g.id);
        }
      } catch (e) {
        busy = false;
        err.textContent = e && e.message ? e.message : 'Couldn’t make the group chat. Try again.';
        err.hidden = false;
        sync();
      }
    });
    const box = h(
      'div',
      { class: 'dialog', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'New group chat' },
      h('button', { class: 'icon-btn dialog-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 16)),
      h('h2', { class: 'dialog-title' }, 'New group chat'),
      h('div', { class: 'dialog-sub' }, `Pick 2 to ${MAX_GROUP} Bots. Each one answers when the message is for it, and stays quiet otherwise.`),
      list,
      nameIn,
      h('div', { class: 'set-note g-note' }, GROUP_COST_NOTE),
      err,
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn', onclick: () => close() }, 'Cancel'), createBtn)
    );
    const close = openModal(box);
  }
  // The + button: a new Bot or a new group chat.
  function newMenu(anchor) {
    menuAt(anchor, [
      { label: 'New Bot', icon: 'plus', kbd: MOD + 'N', action: () => openNewBot() },
      { label: 'New group chat', icon: 'users', kbd: (isMac ? '⌘⇧' : 'Ctrl+Shift+') + 'N', action: () => openNewGroup() },
    ]);
  }
  function openNewBot(preset) {
    const d = defaultsFor('new-' + Math.random());
    const used = new Set([...S.bots.values()].map((b) => lookOf(b).color));
    const freeColor = skinList().filter((k) => k.kind === 'solid' && k.id !== 'ink' && k.id !== 'snow').map((k) => k.id).find((c) => !used.has(c));
    const st = {
      shape: preset ? preset.shape : d.shape,
      color: preset ? preset.color : used.has(d.color) && freeColor ? freeColor : d.color,
    };
    const prev = h('span', { class: 'av', style: { width: '96px', height: '96px' } });
    prev._size = 96;
    paintAvatar(prev, st, 96, { live: true, interactive: true });
    prev.addEventListener('click', () => pokeAvatar(prev));
    const picker = avatarPicker(st, (next) => {
      paintAvatar(prev, next, 96, { live: true, interactive: true });
      pokeAvatar(prev);
    });
    const nameIn = h('input', { class: 'field ne-name', value: preset ? preset.name : '', placeholder: 'Name your Bot', maxlength: '255', 'aria-label': 'Bot name' });
    const err = h('div', { class: 'dialog-error', role: 'alert', hidden: true });
    const createBtn = h('button', { class: 'btn primary', onclick: create }, 'Create');
    let busy = false;
    const sync = () => (createBtn.disabled = busy || !nameIn.value.trim());
    async function create() {
      const name = nameIn.value.trim();
      if (!name || busy) return;
      busy = true;
      sync();
      createBtn.textContent = 'Creating…';
      err.hidden = true;
      entry.dismissable = false;
      try {
        const nb = await api('/api/bots', { method: 'POST', body: { name, color: st.color, shape: st.shape, instructions: preset ? preset.instructions || '' : '', model: '' } });
        entry.dismissable = true;
        close();
        if (nb && nb.id) {
          S.bots.set(nb.id, nb);
          if (!S.msgs.has(nb.id)) S.msgs.set(nb.id, []);
          renderSidebar();
          selectBot(nb.id);
        }
      } catch (e) {
        busy = false;
        entry.dismissable = true;
        createBtn.textContent = 'Create';
        err.textContent = e && e.message ? e.message : 'Couldn’t create the Bot. Try again.';
        err.hidden = false;
        sync();
      }
    }
    nameIn.addEventListener('input', sync);
    nameIn.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && !e.isComposing) create();
    });
    const box = h(
      'div',
      { class: 'dialog', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Create New Bot' },
      h('button', { class: 'icon-btn dialog-close', onclick: () => !busy && close(), 'aria-label': 'Close' }, icon('x', 16)),
      h('h2', { class: 'dialog-title' }, 'Create New Bot'),
      h('div', { class: 'ne-preview' }, prev),
      nameIn,
      h('label', { class: 'f-label' }, 'Color'),
      picker.grid,
      h('label', { class: 'f-label' }, 'Pattern'),
      picker.sw,
      err,
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn', onclick: () => !busy && close() }, 'Cancel'), createBtn)
    );
    const close = openModal(box);
    const entry = modalStack[modalStack.length - 1];
    sync();
    setTimeout(() => nameIn.focus(), 40);
  }
  function openTemplatesFrom(closePrev) {
    closePrev();
    openTemplates();
  }
  function templateCard(t, onclick) {
    return h('button', { class: 'template-card', onclick }, avatar({ shape: t.shape, color: t.color }, 40), h('span', { class: 'tc-name' }, t.name), h('span', { class: 'tc-blurb' }, t.blurb));
  }
  function openTemplates() {
    const close = openModal(
      h(
        'div',
        { class: 'dialog wide', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Bot templates' },
        h('button', { class: 'icon-btn dialog-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 16)),
        h('h2', { class: 'dialog-title' }, 'Bot templates'),
        h('p', { class: 'dialog-body' }, 'Start from a template or make your own. You can change everything later.'),
        h(
          'div',
          { class: 'template-grid' },
          TEMPLATES.map((t) =>
            templateCard(t, () => {
              close();
              openNewBot(t);
            })
          ),
          h(
            'button',
            {
              class: 'template-card',
              onclick: () => {
                close();
                openNewBot();
              },
            },
            h('span', { class: 'blank-plus' }, icon('plus', 18)),
            h('span', { class: 'tc-name' }, 'Start from scratch'),
            h('span', { class: 'tc-blurb' }, 'Pick a name and a character.')
          )
        )
      )
    );
  }

  // ───────────────────────────── search / command palette (⌘K) ─────────────────────────────
  function paletteCommands() {
    return [
      { title: 'New Bot', icon: 'plus', type: 'Action', run: () => openNewBot() },
      { title: 'New group chat', icon: 'users', type: 'Action', run: () => openNewGroup() },
      { title: 'Bot templates', icon: 'gridPlus', type: 'Action', run: () => openTemplates() },
      curBot() && { title: 'Bot settings', sub: curBot().name, icon: 'gear', type: 'Action', run: () => setPanel(true) },
      { title: 'Toggle compact sidebar', icon: 'sidebar', type: 'Action', sub: MOD + 'B', run: () => toggleRail() },
      { title: 'Keyboard shortcuts', icon: 'keyboard', type: 'Action', sub: 'Help', run: () => openShortcuts() },
      { title: 'How openbot works', icon: 'sparkle', type: 'Action', sub: 'Help', run: () => openOnboarding() },
      { title: 'Set up an always-on computer', icon: 'monitor', type: 'Action', sub: 'Help', run: () => openSetupWizard() },
      ...TABS.map(([key, label, ic]) => ({ title: `Settings: ${label}`, icon: ic, type: 'Action', sub: 'Settings', run: () => openSettings(key) })),
    ].filter(Boolean);
  }
  function openSearch(initial = '') {
    if (S.paletteClose) return S.paletteClose();
    const inputEl = h('input', { class: 'search-input', placeholder: 'Search', autocomplete: 'off', spellcheck: 'false', 'aria-label': 'Search' });
    inputEl.value = initial;
    const results = h('div', { class: 'search-results', role: 'listbox' });
    let items = [];
    let sel = 0;
    let seq = 0;
    let msgResults = [];
    let msgLoading = false;
    function go(item) {
      if (item.keep) {
        item.run();
        paint();
        return;
      }
      close();
      item.run();
    }
    const hl = (text, q) => {
      const span = h('span', { class: 'sr-title' });
      const i = q ? text.toLowerCase().indexOf(q) : -1;
      if (i < 0) span.textContent = text;
      else span.append(text.slice(0, i), h('mark', null, text.slice(i, i + q.length)), text.slice(i + q.length));
      return span;
    };
    const match = (s, q) => q.split(/\s+/).every((w) => s.toLowerCase().includes(w));
    function add(it, q) {
      it.el = h(
        'button',
        { class: 'sr-item', role: 'option', onclick: () => go(it), onmousemove: () => setSel(items.indexOf(it)), onmousedown: (e) => e.preventDefault() },
        h('span', { class: 'sr-lead' }, it.bot ? avatar(it.bot, 24) : icon(it.icon || 'bolt', 18)),
        h('span', { class: 'sr-main' }, hl(it.title, q), it.sub ? h('span', { class: 'sr-sub' }, it.sub) : null),
        it.tag ? h('span', { class: 'sr-tag' }, it.tag) : null,
        it.checked ? h('span', { class: 'sr-type' }, icon('check', 14)) : h('span', { class: 'sr-type' }, it.type || '')
      );
      items.push(it);
      results.append(it.el);
    }
    function paint() {
      const q = inputEl.value.trim().toLowerCase();
      results.textContent = '';
      items = [];
      const bots = sortedBots().filter((b) => !q || match(b.name + ' ' + (b.instructions || ''), q));
      const visible = bots.filter((b) => !b.hidden);
      const hidden = bots.filter((b) => b.hidden);
      for (const b of visible) add({ bot: b, title: b.name, sub: (b.last_preview || '').split('\n')[0], type: 'Bot', run: () => selectBot(b.id) }, q);
      const cmds = paletteCommands().filter((c) => !q || match(c.title + ' ' + (c.sub || ''), q));
      for (const c of cmds) add(c, q);
      if (q) {
        for (const b of hidden) add({ bot: b, title: b.name, sub: (b.last_preview || '').split('\n')[0], type: 'Bot', tag: 'Hidden', run: () => selectBot(b.id) }, q);
        for (const m of msgResults.slice(0, 12)) {
          const b = S.bots.get(m.bot_id);
          add({ bot: b || {}, title: snippet(m.text, q), sub: m.role === 'user' ? `You to ${b ? b.name : 'a Bot'}` : `${b ? b.name : 'A Bot'} to you`, type: 'Message', run: () => jumpTo(m) }, q);
        }
      }
      if (!items.length) {
        results.append(
          msgLoading
            ? h('div', { class: 'sr-empty' }, spinner('lg'))
            : h('div', { class: 'sr-empty' }, icon('search', 28), q ? 'No results' : 'No Bots yet')
        );
      }
      setSel(clamp(sel, 0, Math.max(0, items.length - 1)));
    }
    function setSel(i) {
      sel = i;
      items.forEach((it, k) => it.el.classList.toggle('sel', k === i));
      const it = items[i];
      if (it) it.el.scrollIntoView({ block: 'nearest' });
    }
    const fetchMsgs = debounce(async () => {
      const q = inputEl.value.trim();
      const my = ++seq;
      if (!q) {
        msgResults = [];
        msgLoading = false;
        return paint();
      }
      try {
        const r = await api('/api/search?q=' + encodeURIComponent(q));
        if (my !== seq) return;
        msgResults = (r && r.messages) || [];
      } catch {
        if (my !== seq) return;
        msgResults = [];
      }
      msgLoading = false;
      paint();
    }, 220);
    inputEl.addEventListener('input', () => {
      sel = 0;
      msgLoading = !!inputEl.value.trim();
      paint();
      fetchMsgs();
    });
    inputEl.addEventListener('keydown', (e) => {
      const down = e.key === 'ArrowDown' || (isMac && e.ctrlKey && e.key === 'n');
      const up = e.key === 'ArrowUp' || (isMac && e.ctrlKey && e.key === 'p');
      if (down) {
        e.preventDefault();
        setSel(items.length ? (sel + 1) % items.length : 0);
      } else if (up) {
        e.preventDefault();
        setSel(items.length ? (sel - 1 + items.length) % items.length : 0);
      } else if (e.key === 'Enter') {
        e.preventDefault();
        if (items[sel]) go(items[sel]);
      } else if (modKey(e) && /^[1-9]$/.test(e.key)) {
        e.preventDefault();
        const it = items[Number(e.key) - 1];
        if (it) go(it);
      }
    });
    const box = h('div', { class: 'palette', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Jump to' }, h('div', { class: 'pal-head' }, icon('search', 18), inputEl), results);
    const close = openModal(box, { cls: 'top', onClose: () => (S.paletteClose = null) });
    S.paletteClose = close;
    paint();
    if (initial) {
      msgLoading = true;
      fetchMsgs();
    }
    setTimeout(() => {
      inputEl.focus();
      inputEl.setSelectionRange(inputEl.value.length, inputEl.value.length);
    }, 20);
  }
  function snippet(text, q) {
    text = (text || '').replace(/\s+/g, ' ');
    const i = text.toLowerCase().indexOf(q);
    if (i < 0) return text.slice(0, 90);
    const start = Math.max(0, i - 30);
    return (start ? '…' : '') + text.slice(start, start + 100);
  }
  async function jumpTo(m) {
    await selectBot(m.bot_id);
    const c = S.nodes.get(m.id);
    if (c) {
      c.el.scrollIntoView({ block: 'center', behavior: 'smooth' });
      c.el.classList.remove('flash');
      void c.el.offsetWidth;
      c.el.classList.add('flash');
      setTimeout(() => c.el.classList.remove('flash'), 2600);
    }
  }

  // ───────────────────────────── account menu + shortcuts sheet ─────────────────────────────
  function accountMenu() {
    menuAt(
      $('#meBtn'),
      [
        { label: 'Settings', icon: 'gear', kbd: MOD + ',', action: () => openSettings('general') },
        { label: 'Computer', icon: 'monitor', action: () => openSettings('computer') },
        { label: 'Usage & Billing', icon: 'bars', action: () => openSettings('usage') },
        '-',
        { label: 'Bot templates', icon: 'gridPlus', action: () => openTemplates() },
        { label: 'Keyboard shortcuts', icon: 'keyboard', kbd: MOD + '/', action: () => openShortcuts() },
        { label: 'How openbot works', icon: 'sparkle', action: () => openOnboarding() },
      ],
      'top-start'
    );
  }
  function openShortcuts() {
    const rows = [
      ['Jump to / search', MOD + 'K'],
      ['New Bot', MOD + 'N'],
      ['New group chat', (isMac ? '⌘⇧' : 'Ctrl+Shift+') + 'N'],
      ['Send', '↵'],
      ['New line', '⇧↵'],
      ['Go to Bot 1–9', MOD + '1…9'],
      ['Previous / next Bot', '⌥↑ / ⌥↓'],
      ['Compact sidebar', MOD + 'B'],
      ['Bot settings', isMac ? '⌘⇧,' : 'Ctrl+Shift+,'],
      ['Settings', MOD + ','],
      ['Focus chat', MOD + 'I'],
      ['Close panel or dialog', 'Esc'],
    ];
    const close = openModal(
      h(
        'div',
        { class: 'dialog', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Keyboard shortcuts' },
        h('button', { class: 'icon-btn dialog-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 16)),
        h('h2', { class: 'dialog-title' }, 'Keyboard shortcuts'),
        h('div', { class: 'kbd-grid' }, rows.map(([a, b]) => [h('span', null, a), h('span', null, b)]))
      )
    );
  }
  // ───────────────────────────── settings (settings-and-bots.md 1.x, 2.1, 3.7, 6) ─────────────────────────────
  const TABS = [
    ['general', 'General', 'gear'],
    ['computer', 'Computer', 'monitor'],
    ['permissions', 'Permissions', 'shield'],
    ['vault', 'Vault', 'vault'],
    ['connections', 'Connections', 'plug'],
    ['usage', 'Usage & Billing', 'bars'],
    ['updates', 'Updates', 'updates'],
  ];
  const TAB_ALIAS = { account: 'usage' };
  const SAVE_ERR = 'Couldn’t save. Try again.';
  const USAGE_URL = 'https://claude.ai/settings/usage';
  // The desktop app's menu bar (openbot › Settings…, Connect to Remote Computer…) calls this.
  window.openbotOpenSettings = (tab) => openSettings(tab);
  function openSettings(tab) {
    tab = TAB_ALIAS[tab] || tab;
    if (S.settingsUI) {
      if (tab) S.settingsUI.tab = tab;
      rerenderSettings(true);
      return;
    }
    const nav = h('nav', { class: 'set-nav', 'aria-label': 'Settings sections' });
    const body = h('div', { class: 'set-body' });
    const main = h('div', { class: 'set-main' }, body);
    const box = h(
      'div',
      { class: 'settings', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'openbot settings' },
      nav,
      main,
      h('button', { class: 'icon-btn set-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 20))
    );
    const close = openModal(box, {
      cls: 'settings-overlay',
      onClose: () => {
        S.settingsUI = null;
        S.connDraft = null;
      },
    });
    S.settingsUI = { tab: tab || 'general', nav, body, main, close, errs: new Set(), ruleEdit: null, vaultForm: null, upd: {} };
    S.connDraft = null;
    rerenderSettings(true);
    refreshSettingsData();
  }
  async function refreshSettingsData() {
    const jobs = [home('/api/connection').then((c) => (S.conn = Object.assign({ mode: 'local', url: '', token: '' }, c || {})))];
    if (S.booted) {
      jobs.push(
        api('/api/computer').then((c) => (S.computer = c)),
        api('/api/usage').then((u) => {
          S.usage = u;
          renderUsageBanner();
        }),
        api('/api/settings').then(
          (s) => {
            S.settings = s;
            S.settingsLoadErr = false;
          },
          (e) => {
            S.settingsLoadErr = true;
            throw e;
          }
        )
      );
    }
    await Promise.allSettled(jobs);
    if (S.settingsUI) rerenderSettings();
  }
  function rerenderSettings(force) {
    const ui = S.settingsUI;
    if (!ui) return;
    const ae = document.activeElement;
    if (!force && ae && ui.body.contains(ae) && /INPUT|TEXTAREA|SELECT/.test(ae.tagName)) return;
    ui.nav.textContent = '';
    for (const [key, label, ic] of TABS) {
      ui.nav.append(
        h(
          'button',
          {
            class: 'set-nav-item' + (ui.tab === key ? ' active' : ''),
            'aria-current': ui.tab === key ? 'page' : null,
            onclick: () => {
              if (ui.tab === key) return;
              ui.tab = key;
              ui.ruleEdit = null;
              ui.vaultForm = null;
              ui.main.scrollTop = 0;
              rerenderSettings(true);
            },
          },
          icon(ic, 20),
          h('span', null, label)
        )
      );
    }
    const st = ui.main.scrollTop;
    ui.body.textContent = '';
    const tabDef = TABS.find((t) => t[0] === ui.tab) || TABS[0];
    ui.body.append(h('h1', { class: 'set-title' }, tabDef[1]));
    const render = { general: tabGeneral, computer: tabComputer, permissions: tabPermissions, vault: tabVault, connections: tabConnections, usage: tabUsage, updates: tabUpdates }[tabDef[0]];
    try {
      ui.body.append(render());
    } catch (e) {
      console.error(e);
      ui.body.append(h('div', { class: 'set-note' }, 'Couldn’t show this page: ' + e.message));
    }
    ui.main.scrollTop = st;
  }

  // settings building blocks
  const section = (label, ...kids) => h('div', { class: 'set-section' }, label ? h('div', { class: 'set-label' }, label) : null, ...kids);
  const card = (...rows) => h('div', { class: 'set-card' }, ...rows);
  const note = (...kids) => h('div', { class: 'set-note' }, ...kids);
  // row(title, control, sub, cls, anchor): a failed save on `anchor` swaps the description for "Couldn't save. Try again."
  function row(title, control, sub, cls = '', anchor = null) {
    const err = anchor && S.settingsUI && S.settingsUI.errs.has(anchor);
    return h(
      'div',
      { class: 'set-row ' + cls, id: anchor ? 'set-' + anchor : null },
      h('div', { class: 'set-row-text' }, h('div', { class: 'set-row-title' }, title), err ? h('div', { class: 'set-row-sub err', role: 'alert' }, SAVE_ERR) : sub ? h('div', { class: 'set-row-sub' }, sub) : null),
      control ? h('div', { class: 'set-row-ctl' }, control) : null
    );
  }
  function toggleEl(checked, onchange) {
    const i = h('input', { type: 'checkbox', class: 'toggle', role: 'switch' });
    i.checked = !!checked;
    i.addEventListener('change', () => onchange(i.checked));
    return i;
  }
  function selectEl(value, options, onchange, cls = '') {
    const s = h(
      'select',
      { class: 'pill-select ' + cls },
      options.map(([v, l]) => h('option', { value: v }, l))
    );
    s.value = value == null ? '' : value;
    s.addEventListener('change', () => onchange(s.value));
    return s;
  }
  const btn = (label, onclick, cls = '') => h('button', { class: 'btn ' + cls, onclick }, label);
  const pillBtn = (label, onclick, attrs = {}) => h('button', Object.assign({ class: 'btn pill', onclick }, attrs), label);
  function markSave(anchor, ok) {
    const ui = S.settingsUI;
    if (!ui || !anchor) return;
    if (ok) ui.errs.delete(anchor);
    else ui.errs.add(anchor);
  }
  async function putSettings(patch, anchor) {
    try {
      const s = await api('/api/settings', { method: 'PUT', body: patch });
      if (s) S.settings = s;
      markSave(anchor, true);
      rerenderSettings(true);
      return true;
    } catch (e) {
      if (anchor) markSave(anchor, false);
      else toast(SAVE_ERR, 'error');
      rerenderSettings(true);
      return false;
    }
  }
  const notConnected = () => note('Not connected to a computer right now. Check the Computer page.');
  function openExternal(url) {
    // In the desktop app, open links in the normal web browser.
    const T = window.__TAURI_INTERNALS__;
    if (T && /^https?:\/\//.test(url)) {
      T.invoke('open_url', { url }).catch(() => copyText(url, 'Link copied. Paste it in your browser.'));
      return;
    }
    const w = window.open(url, '_blank', 'noopener');
    if (!w) copyText(url, 'Link copied. Paste it in your browser.');
  }

  // searchable dropdown (Timezone, 1.3 §6)
  function searchSelect({ value, options, placeholder, label, closeLabel, onChange }) {
    const cur = () => options().find((o) => o.value === value) || options()[0];
    const b = h('button', { class: 'pill-btn', 'aria-haspopup': 'listbox', 'aria-label': label }, h('span', { class: 'pill-btn-text' }, cur().label), icon('chevronDown', 12, '', 2));
    b.addEventListener('click', () => {
      closeMenu();
      const opts = options();
      const q = h('input', { class: 'sp-search', placeholder, 'aria-label': placeholder, spellcheck: 'false' });
      const list = h('div', { class: 'sp-list', role: 'listbox', 'aria-label': label });
      const pop = h('div', { class: 'menu search-pop' }, h('div', { class: 'sp-head' }, icon('search', 15), q, h('button', { class: 'ghost-btn sm', 'aria-label': closeLabel, onclick: () => closeMenu() }, icon('x', 13))), list);
      let shown = [];
      let sel = 0;
      const paint = () => {
        const t = q.value.trim().toLowerCase();
        shown = opts.filter((o) => !t || o.label.toLowerCase().includes(t) || (o.search || '').includes(t)).slice(0, 300);
        list.textContent = '';
        if (!shown.length) list.append(h('div', { class: 'p-empty' }, `No matches for “${q.value.trim()}”`));
        shown.forEach((o, i) =>
          list.append(
            h(
              'button',
              {
                class: 'menu-item' + (i === sel ? ' kb' : ''),
                role: 'option',
                'aria-selected': o.value === value ? 'true' : 'false',
                onclick: () => pick(o),
                onmousemove: () => {
                  if (sel !== i) {
                    sel = i;
                    for (const [j, el] of [...list.children].entries()) el.classList.toggle('kb', j === i);
                  }
                },
              },
              h('span', null, o.label),
              o.value === value ? h('span', { class: 'kbd' }, icon('check', 14)) : o.meta ? h('span', { class: 'kbd' }, o.meta) : null
            )
          )
        );
      };
      const pick = (o) => {
        closeMenu();
        value = o.value;
        b.firstChild.textContent = o.label;
        onChange(o.value);
      };
      q.addEventListener('input', () => {
        sel = 0;
        paint();
      });
      q.addEventListener('keydown', (e) => {
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          e.preventDefault();
          sel = clamp(sel + (e.key === 'ArrowDown' ? 1 : -1), 0, shown.length - 1);
          [...list.children].forEach((el, j) => el.classList.toggle('kb', j === sel));
          list.children[sel] && list.children[sel].scrollIntoView({ block: 'nearest' });
        } else if (e.key === 'Enter' && shown[sel]) {
          e.preventDefault();
          pick(shown[sel]);
        }
      });
      paint();
      document.body.append(pop);
      const a = b.getBoundingClientRect();
      const r = pop.getBoundingClientRect();
      pop.style.left = clamp(a.right - r.width, 8, innerWidth - r.width - 8) + 'px';
      pop.style.top = (a.bottom + 6 + r.height > innerHeight - 8 ? Math.max(8, a.top - r.height - 6) : a.bottom + 6) + 'px';
      openMenuEl = pop;
      setTimeout(() => q.focus(), 0);
    });
    return b;
  }
  function timezoneOptions() {
    const auto = Intl.DateTimeFormat().resolvedOptions().timeZone || '';
    let zones = [];
    try {
      zones = Intl.supportedValuesOf ? Intl.supportedValuesOf('timeZone') : [];
    } catch {}
    const now = new Date();
    const out = [{ value: '', label: auto ? `Auto-detect (${auto})` : 'Auto-detect' }];
    for (const z of zones) {
      let meta = '';
      try {
        meta = now.toLocaleTimeString([], { timeZone: z, hour: 'numeric', minute: '2-digit' });
      } catch {}
      out.push({ value: z, label: z.replace(/_/g, ' '), meta, search: z.toLowerCase() });
    }
    return out;
  }

  // ── General (1.3) ──
  function tabGeneral() {
    const name = (S.prefs.userName || '').trim();
    const claude = (S.computer && S.computer.claude) || {};
    const nameIn = h('input', { class: 'field inline-field strong', value: name, placeholder: 'Your name', maxlength: '40', 'aria-label': 'Your name' });
    const commitName = () => {
      const v = nameIn.value.trim();
      if (v === (S.prefs.userName || '')) return;
      S.prefs.userName = v;
      savePrefs();
      renderMe();
      rerenderSettings(true);
    };
    nameIn.addEventListener('blur', commitName);
    nameIn.addEventListener('keydown', (e) => e.key === 'Enter' && nameIn.blur());
    const signedIn = !!(claude.logged_in && claude.email);
    const signOut = async () => {
      const ok = await confirmDialog({
        title: 'Sign out?',
        body: 'Signing out logs Claude Code out on the computer where your Bots run. Your Bots and chats stay here and are back when you sign in again.',
        confirmText: 'Sign Out',
        danger: true,
      });
      if (!ok) return;
      try {
        await api('/api/claude/logout', { method: 'POST' });
        refreshSettingsData();
      } catch {
        toast('Couldn’t sign out. Run claude /logout in a terminal on that computer.', 'error');
      }
    };
    const accountCard = card(
      h(
        'div',
        { class: 'set-row account-row' },
        h('span', { class: 'acct-circle' }, ((name || claude.email || 'You')[0] || 'Y').toUpperCase()),
        h(
          'div',
          { class: 'set-row-text' },
          nameIn,
          signedIn
            ? h('div', { class: 'acct-email' }, h('span', null, claude.email), h('button', { class: 'ghost-btn sm', 'aria-label': 'Copy email address', 'data-tip': 'Copy email address', onclick: () => copyText(claude.email) }, icon('copy', 14)))
            : h('div', { class: 'set-row-sub' }, claude.installed === false ? 'Not signed in. Install Claude Code on the computer where Bots run.' : 'Not signed in. Run claude in a terminal on that computer to sign in.')
        ),
        signedIn && h('div', { class: 'set-row-ctl' }, pillBtn('Sign Out', signOut))
      )
    );
    const accentVal = S.prefs.accent === 'none' ? 'none' : 'bot';
    const s = S.settings;
    const tz = searchSelect({
      value: (s && s.timezone) || '',
      options: timezoneOptions,
      placeholder: 'Search timezone',
      label: 'Timezone',
      closeLabel: 'Close timezone picker',
      // Saved on the computer where Bots run: routine times use it.
      onChange: (v) => putSettings({ timezone: v }),
    });
    return h(
      'div',
      null,
      section('Account', accountCard),
      pushSection(),
      section(
        'Appearance',
        card(
          row(
            'Accent',
            selectEl(accentVal, [['none', 'Black'], ['bot', 'Bot color']], (v) => {
              S.prefs.accent = v;
              savePrefs();
              applyTint();
            })
          ),
          row('Language', selectEl('system', [['system', 'Follow System']], () => {})),
          row(
            'Check Spelling While Typing',
            toggleEl(S.prefs.spellcheck !== false, (v) => {
              S.prefs.spellcheck = v;
              savePrefs();
              updateComposer();
            })
          )
        )
      ),
      section(
        'Bot',
        card(
          row('Timezone', tz),
          s ? row('Model', selectEl(s.default_model || '', modelOptions(modelName(s.model_default || 'claude-sonnet-5-5') + ' (default)', s.default_model), (v) => putSettings({ default_model: v }, 'model')), 'Used by every Bot that doesn’t pick its own', '', 'model') : row('Model', null, 'Connect to a computer to change this.'),
          s ? row('Effort', selectEl(s.effort || 'xhigh', EFFORTS, (v) => putSettings({ effort: v }, 'effort')), 'How hard Bots think before answering. Higher is smarter but slower and uses more of your plan', '', 'effort') : null,
          row(
            'Show Tool Activity',
            toggleEl(S.prefs.showTools === true, (v) => {
              S.prefs.showTools = v;
              savePrefs();
              renderMessages(false);
            }),
            'Show each command and file edit as its own line in the chat'
          )
        )
      ),
      section(null, card(row('Import Memory to openbot', pillBtn('Import', () => openImportMemory()), 'Transfer your preferences and context from another AI assistant', '', 'import-memory'))),
      section(
        'Help',
        card(
          row('How openbot works', pillBtn('Show tour', () => (S.settingsUI && S.settingsUI.close(), openOnboarding())), 'Replay the short welcome tour'),
          TAURI || onThisComputer() ? row('Set up an always-on computer', pillBtn('Start setup', () => openSetupWizard()), 'Keep your Bots working 24/7 on a PC that stays on') : null
        )
      )
    );
  }
  const IMPORT_PROMPT = [
    "Export all of my stored memories and any context you've learned about me from past conversations. Preserve my words verbatim where possible, especially for instructions and preferences.",
    '',
    '## Categories (output in this order):',
    '',
    '1. **Instructions**: Rules I\'ve explicitly asked you to follow going forward — tone, format, style, "always do X", "never do Y", and corrections to your behavior. Only include rules from stored memories, not from conversations.',
    '',
    '2. **Identity**: Name, age, location, education, family, relationships, languages, and personal interests.',
    '',
    '3. **Career**: Current and past roles, companies, and general skill areas.',
    '',
    '4. **Projects**: Projects I meaningfully built or committed to. Ideally ONE entry per project. Include what it does, current status, and any key decisions. Use the project name or a short descriptor as the first words of the entry.',
    '',
    '5. **Preferences**: Opinions, tastes, and working-style preferences that apply broadly.',
    '',
    '## Format:',
    '',
    'Use section headers for each category. Within each category, list one entry per line, sorted by oldest date first. Format each line as:',
    '',
    '[YYYY-MM-DD] - Entry content here.',
    '',
    'If no date is known, use [unknown] instead.',
    '',
    '## Output:',
    '- Wrap the entire export in a single code block for easy copying.',
    '- After the code block, state whether this is the complete set or if more remain.',
  ].join('\n');
  function openImportMemory() {
    const copyBtn = h('button', { class: 'btn small' }, icon('copy', 14), h('span', null, 'Copy'));
    copyBtn.addEventListener('click', () => {
      copyText(IMPORT_PROMPT, 'Copied');
      copyBtn.lastChild.textContent = 'Copied';
      setTimeout(() => copyBtn.isConnected && (copyBtn.lastChild.textContent = 'Copy'), 2000);
    });
    const ta = h('textarea', { class: 'field', rows: '6', placeholder: 'Paste your memory details here', 'aria-label': 'Paste your memory details here' });
    const err = h('div', { class: 'dialog-error', role: 'alert', hidden: true });
    const addBtn = h('button', { class: 'btn primary', disabled: true }, 'Add to Memory');
    ta.addEventListener('input', () => (addBtn.disabled = !ta.value.trim()));
    addBtn.addEventListener('click', async () => {
      const raw = ta.value.trim();
      err.hidden = true;
      if (raw.length > 60000) {
        err.textContent = 'That’s too long. Paste a shorter export.';
        err.hidden = false;
        return;
      }
      // keep the entries; drop code fences, headings and the trailing "this is the complete set" line
      const text = raw
        .split('\n')
        .map((l) => l.trim().replace(/^\[unknown\]\s*-\s*/i, ''))
        .filter((l) => l && !/^```/.test(l) && !/^#{1,6}\s/.test(l) && !/^\d+\.\s*\*\*[^*]+\*\*:?\s*$/.test(l) && !/^\*\*[^*]+\*\*:?$/.test(l))
        .join('\n');
      addBtn.disabled = true;
      addBtn.textContent = 'Adding…';
      try {
        const r = await api('/api/memories/import', { method: 'POST', body: { text } });
        close();
        toast(r && r.imported != null ? `Added ${r.imported} ${r.imported === 1 ? 'memory' : 'memories'}` : 'Added to memory');
      } catch (e) {
        addBtn.disabled = false;
        addBtn.textContent = 'Add to Memory';
        err.textContent = e && e.network ? 'Couldn’t add to memory. Check your connection and try again.' : e.message;
        err.hidden = false;
      }
    });
    const box = h(
      'div',
      { class: 'dialog wide', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Import Memory to openbot' },
      h('button', { class: 'icon-btn dialog-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 16)),
      h('h2', { class: 'dialog-title' }, 'Import Memory to openbot'),
      h('div', { class: 'im-step' }, h('span', { class: 'im-num' }, '1'), h('span', null, 'Copy this prompt into a chat with your other AI assistant')),
      h('div', { class: 'im-prompt' }, h('pre', null, IMPORT_PROMPT), h('div', { class: 'im-copy' }, copyBtn)),
      h('div', { class: 'im-step' }, h('span', { class: 'im-num' }, '2'), h('span', null, 'Paste the results below to add them to openbot’s memory')),
      ta,
      err,
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn', onclick: () => close() }, 'Cancel'), addBtn)
    );
    const close = openModal(box);
  }

  // ── Computer (1.7.2) ──
  const EXEC_OPTS = [
    ['allow_all', 'Always allow'],
    ['ask', 'Ask every time'],
    ['never', 'Never allow'],
  ];
  function tabComputer() {
    const c = S.computer;
    const conn = S.conn;
    if (!S.connDraft) S.connDraft = { mode: conn.mode === 'remote' ? 'remote' : 'local', url: conn.url || '', code: '', token: '', useToken: false, result: null, busy: false };
    const d = S.connDraft;
    const out = h('div');
    const s = S.settings;

    // Your computer › Execution
    if (s) {
      const pm = s.permission_mode === 'allow_all' || s.permission_mode === 'never' ? s.permission_mode : 'ask';
      out.append(
        section(
          'Your computer',
          card(
            row(
              conn.mode === 'remote' && c && c.hostname ? `Execution on ${c.hostname}` : 'Execution on this computer',
              selectEl(pm, EXEC_OPTS, (v) => putSettings({ permission_mode: v }, 'local-execution'), 'ghost'),
              'Let openbot open files and run tasks on your computer. Auto-review still checks everything first.',
              '',
              'local-execution'
            ),
            // Windows computers only: run Claude Code inside WSL Linux, where its sandbox works.
            s.wsl_available
              ? row(
                  'Where Bots run commands',
                  selectEl(
                    s.runner === 'wsl' ? 'wsl' : 'native',
                    [
                      ['native', 'Directly on Windows'],
                      ['wsl', `In Linux (WSL: ${s.wsl_distro || 'Ubuntu'}), sandboxed`],
                    ],
                    (v) => putSettings({ runner: v }, 'bot-runner'),
                    'ghost'
                  ),
                  'In Linux, commands run in Claude Code’s sandbox and can only change files in the Bot’s folder. Needs Claude Code installed and logged in inside WSL.',
                  '',
                  'bot-runner'
                )
              : null,
            screensRow()
          )
        )
      );
    } else out.append(section('Your computer', card(row('Execution on this computer', null, S.settingsLoadErr ? 'Couldn’t load this setting' : 'Loading…'))));

    // where bots run (a phone or other browser is paired with this computer and can't switch it)
    if (!TAURI && !onThisComputer()) {
      out.append(section('Where Bots run', card(row(h('span', { class: 'comp-name' }, icon('monitor', 16), computerName()), null, `Your Bots run on this computer. This ${browserDeviceName() === 'Web browser' ? 'browser' : browserDeviceName()} is paired with it.`))));
    } else {
    const seg = h(
      'div',
      { class: 'segmented' },
      [
        ['local', 'This computer'],
        ['remote', 'Remote computer'],
      ].map(([v, l]) =>
        h(
          'button',
          {
            class: d.mode === v ? 'on' : '',
            onclick: () => {
              d.mode = v;
              d.result = null;
              rerenderSettings(true);
            },
          },
          l
        )
      )
    );
    const linkOk = S.wsOk && (!S.link || S.link === 'ok');
    const current =
      conn.mode === 'remote'
        ? row(
            h('span', { class: 'comp-name' }, icon('monitor', 16), computerName()),
            h('div', { class: 'secret-ctl' }, h('span', { class: 'status-pill ' + (linkOk ? 'on' : 'off') }, h('i'), linkOk ? 'Online' : S.link === 'unpaired' ? 'Not paired' : 'Can’t reach'), pillBtn(`Switch to ${thisDevice()}`, () => useThisDevice())),
            `Your Bots run here · ${hostOf(conn.url)}`
          )
        : row(
            h('span', { class: 'comp-name' }, icon('monitor', 16), ThisDevice()),
            pillBtn('Set up an always-on computer', () => openSetupWizard()),
            `Your Bots run here. They pause when ${thisDevice()} sleeps.`
          );
    const runsOn = card(current, row('Bots run on', seg, conn.mode === 'remote' ? `Connected to ${hostOf(conn.url)}` : `Bots are using ${thisDevice()}.`));
    if (d.mode === 'remote') {
      // Pair with another computer: its private address + a 6-digit code made on that computer.
      const urlIn = h('input', { class: 'field', placeholder: 'https://my-pc.your-tailnet.ts.net', value: d.url, spellcheck: 'false', autocomplete: 'off', 'aria-label': 'Computer address' });
      const codeIn = h('input', { class: 'field mono', placeholder: '123 456', value: d.code, inputmode: 'numeric', autocomplete: 'one-time-code', maxlength: '7', 'aria-label': 'Pairing code' });
      const tokIn = h('input', { class: 'field', type: 'password', placeholder: 'Token', value: d.token, autocomplete: 'off', 'aria-label': 'Token' });
      urlIn.addEventListener('input', () => (d.url = urlIn.value));
      codeIn.addEventListener('input', () => (d.code = codeIn.value));
      tokIn.addEventListener('input', () => (d.token = tokIn.value));
      const resultEl = d.result ? h('div', { class: 'conn-result ' + (d.result.ok ? 'ok' : 'bad') }, d.result.ok ? icon('check', 15) : icon('alert', 15), h('span', null, d.result.text)) : null;
      const finish = async (url, token, name) => {
        S.conn = await home('/api/connection', { method: 'PUT', body: { mode: 'remote', url, token } });
        if (name) {
          S.prefs.remoteNames = Object.assign({}, S.prefs.remoteNames, { [S.conn.url || url]: name });
          await savePrefsNow();
        }
        S.connDraft = null;
        S.current = null;
        await boot();
        toast(`Connected to ${hostOf(url)}`);
      };
      const connect = async () => {
        const url = normalizeUrl(d.url);
        if (!url) return urlIn.focus();
        const code = (d.code || '').replace(/\D/g, '');
        if (!d.useToken && code.length !== 6) {
          d.result = { ok: false, text: 'Enter the 6-digit pairing code shown on that computer.' };
          rerenderSettings(true);
          return;
        }
        if (d.useToken && !d.token.trim()) return tokIn.focus();
        d.busy = true;
        d.result = null;
        rerenderSettings(true);
        try {
          if (d.useToken) {
            const r = await home('/api/connection/test', { method: 'POST', body: { url, token: d.token.trim() } });
            if (!r || !r.ok) throw new Error((r && r.error) || 'Couldn’t connect');
            await finish(url, d.token.trim());
          } else {
            const r = await home('/api/connection/pair', { method: 'POST', body: { url, code } });
            if (!r || !r.ok) throw new Error((r && r.error) || 'Couldn’t pair');
            await finish(url, r.token, r.computer && r.computer.hostname);
          }
        } catch (e) {
          d.busy = false;
          d.result = { ok: false, text: e.message };
          rerenderSettings(true);
        }
      };
      const insecure = /^http:\/\//i.test(normalizeUrl(d.url)) && !/^http:\/\/(127\.0\.0\.1|localhost)(:|\/|$)/i.test(normalizeUrl(d.url));
      runsOn.append(
        h(
          'div',
          { class: 'set-row stack' },
          h('div', { class: 'set-row-sub' }, 'On the other computer, run the openbot host setup. It shows a private address and a 6-digit pairing code. Enter both here.'),
          h('label', { class: 'f-label' }, 'Address'),
          urlIn,
          insecure && h('div', { class: 'conn-result bad' }, icon('alert', 15), h('span', null, 'This address isn’t encrypted. Use the https://…ts.net address from Tailscale Serve, or an SSH tunnel to http://127.0.0.1.')),
          d.useToken ? h('label', { class: 'f-label' }, 'Token') : h('label', { class: 'f-label' }, 'Pairing code'),
          d.useToken ? tokIn : codeIn,
          resultEl,
          h(
            'div',
            { class: 'row-actions' },
            h(
              'button',
              {
                class: 'link-btn',
                onclick: () => {
                  d.useToken = !d.useToken;
                  d.result = null;
                  rerenderSettings(true);
                },
              },
              d.useToken ? 'Use a pairing code instead' : 'Use a token instead (SSH tunnel)'
            ),
            btn(d.busy ? 'Connecting…' : 'Connect', connect, 'primary')
          )
        )
      );
    } else if (conn.mode === 'remote') {
      runsOn.append(
        row(
          '',
          btn(`Use ${thisDevice()}`, () => useThisDevice(), 'primary'),
          `Switch back so bots run on this computer instead of ${hostOf(conn.url)}.`
        )
      );
    }
    out.append(section('Where Bots run', runsOn));
    }

    // status
    if (c) {
      const claude = c.claude || {};
      const memPct = c.mem_total ? clamp((c.mem_used / c.mem_total) * 100, 0, 100) : 0;
      out.append(
        section(
          'Status',
          card(
            row('Status', h('span', { class: 'status-pill ' + (S.wsOk ? 'on' : 'off') }, h('i'), S.wsOk ? 'Online' : 'Offline'), conn.mode === 'remote' ? hostOf(S.apiBase) : 'This computer'),
            row('Name', h('span', { class: 'val' }, c.hostname || '—')),
            row('System', h('span', { class: 'val' }, [c.os, c.os_version].filter(Boolean).join(' ') + (c.arch ? ` · ${c.arch}` : ''))),
            row('Up for', h('span', { class: 'val' }, fmtDuration(c.uptime_secs))),
            row('Processor', h('span', { class: 'val' }, c.cpu_count ? `${c.cpu_count} cores` : '—')),
            h(
              'div',
              { class: 'set-row' },
              h('div', { class: 'set-row-text' }, h('div', { class: 'set-row-title' }, 'Memory')),
              h('div', { class: 'set-row-ctl mem' }, h('div', { class: 'meter' }, h('div', { class: 'meter-fill', style: { width: memPct + '%' } })), h('span', { class: 'val small' }, `${fmtBytes(c.mem_used)} of ${fmtBytes(c.mem_total)}`))
            ),
            row(
              'Claude Code',
              h('span', { class: 'val' }, claude.installed ? `Version ${(claude.version || '').replace(/^v/, '').split(' ')[0] || '?'}` : 'Not installed'),
              claude.installed ? (claude.logged_in ? `Signed in as ${claude.email || 'your Claude account'}` : 'Not signed in. Open a terminal on that computer and run: claude') : 'Install Claude Code on that computer, then run: claude'
            ),
            row('Listening on', h('span', { class: 'val mono' }, c.listen || '—'), c.is_remote_capable ? 'Warning: any computer on the network can reach this. Safer: listen on 127.0.0.1 and use Tailscale Serve.' : 'Private: only this computer, plus devices you pair through Tailscale Serve or an SSH tunnel.'),
            row('Data folder', h('span', { class: 'val mono ellipsis', title: c.data_dir || '' }, c.data_dir || '—'))
          )
        )
      );
    } else out.append(section('Status', card(row('Status', h('span', { class: 'status-pill off' }, h('i'), 'Offline'), 'Not connected'))));

    // paired devices (devices that can use the computer where bots run)
    out.append(pairedDevicesSection());

    // plugins (custom MCP servers, 4.8)
    if (s) {
      const servers = s.mcp_servers || [];
      out.append(
        h(
          'div',
          { class: 'set-section' },
          h('div', { class: 'set-head' }, h('div', { class: 'set-head-text' }, h('h3', { class: 'set-h3' }, 'Plugins'), h('div', { class: 'set-explain' }, 'Custom MCP servers every Bot can use')), pillBtn('Add plugin', () => openMcpForm())),
          servers.length
            ? card(
                servers.map((m) =>
                  row(
                    h('span', { class: 'mono' }, m.name),
                    h(
                      'button',
                      {
                        class: 'ghost-btn',
                        'aria-label': `Remove ${m.name}`,
                        'data-tip': 'Remove',
                        onclick: async () => {
                          const ok = await confirmDialog({ title: `Remove “${m.name}”?`, body: 'Bots can no longer use this plugin.', confirmText: 'Remove', danger: true });
                          if (!ok) return;
                          try {
                            S.settings = await api('/api/mcp-servers/' + encodeURIComponent(m.name), { method: 'DELETE' });
                          } catch (e) {
                            fail(e);
                          }
                          rerenderSettings(true);
                        },
                      },
                      icon('trash', 16)
                    ),
                    m.type === 'stdio' ? `Command · ${[m.command, ...(m.args || [])].join(' ')}` : `Remote HTTPS · ${m.url || ''}`
                  )
                )
              )
            : null
        )
      );
    }

    // workspace folder
    if (s) {
      const wsIn = h('input', { class: 'field mono', value: s.workspace_root || '', spellcheck: 'false' });
      out.append(
        section(
          'Workspace',
          card(
            h(
              'div',
              { class: 'set-row stack' },
              h('div', { class: 'set-row-title' }, 'Bot folders'),
              h('div', { class: 'set-row-sub' }, 'Each bot gets its own folder inside this one, where it keeps files it makes.'),
              h(
                'div',
                { class: 'inline-form' },
                wsIn,
                btn('Save', () => putSettings({ workspace_root: wsIn.value.trim() }).then((ok) => ok && toast('Workspace folder saved'))),
                S.conn.mode !== 'remote' && btn('Open', () => api('/api/open-folder', { method: 'POST', body: { path: s.workspace_root } }).catch(fail))
              )
            )
          )
        )
      );
    }

    out.append(
      section(
        'Run openbot 24/7 on another PC',
        card(
          h(
            'div',
            { class: 'set-row stack help' },
            h(
              'ol',
              { class: 'steps' },
              h('li', null, 'On the other PC (for example your Windows PC), install Claude Code and sign in by running ', h('code', null, 'claude'), ' once.'),
              h('li', null, 'Install Tailscale on that PC and on this computer, and sign in to the same account on both.'),
              h('li', null, 'On the PC, run the openbot host setup (', h('code', null, 'install-host.ps1'), '). It keeps openbot private and prints a ', h('code', null, 'https://…ts.net'), ' address and a 6-digit pairing code.'),
              h('li', null, 'Choose “Remote computer” above, enter the address and the code, then press Connect.')
            ),
            h('div', { class: 'set-row-sub' }, 'Only your own devices can reach the PC, and everything is encrypted. Never open port 7777 on your router or use Tailscale Funnel. Set the PC to never sleep so your bots stay online.'),
            h('div', { class: 'row-actions' }, btn('Open the step-by-step guide', () => openSetupWizard(), 'primary'))
          )
        )
      )
    );
    return out;
  }
  function pairedDevicesSection() {
    const wrap = h('div', { class: 'set-section' });
    // Reset when the app switches to a different computer.
    if (!S.devicesUI || S.devicesUI.base !== S.apiBase) S.devicesUI = { base: S.apiBase, list: null, err: null, code: null, loading: false };
    const st = S.devicesUI;
    const load = async () => {
      st.loading = true;
      try {
        st.list = await api('/api/devices');
        st.err = null;
      } catch (e) {
        st.err = e.message;
      }
      st.loading = false;
      rerenderSettings(true);
    };
    if (!st.list && !st.loading && !st.err) load();
    const makeCode = async () => {
      try {
        st.code = await api('/api/pair/code', { method: 'POST' });
        rerenderSettings(true);
      } catch (e) {
        fail(e);
      }
    };
    const where = S.conn.mode === 'remote' ? hostOf(S.conn.url) : 'this computer';
    wrap.append(
      h(
        'div',
        { class: 'set-head' },
        h('div', { class: 'set-head-text' }, h('h3', { class: 'set-h3' }, 'Paired devices'), h('div', { class: 'set-explain' }, `Devices that can use the bots on ${where}. Remove any you don’t recognize.`)),
        pillBtn('Pair a new device', makeCode)
      )
    );
    const rows = [];
    if (st.code) {
      const left = Math.max(0, Math.round((st.code.expires_at - Date.now()) / 60000));
      rows.push(
        row(
          h('span', { class: 'mono pair-code' }, st.code.code.slice(0, 3) + ' ' + st.code.code.slice(3)),
          btn('Done', () => {
            st.code = null;
            load();
          }),
          `Enter this code on the new device. It works once, for ${left} more minute${left === 1 ? '' : 's'}.`
        )
      );
      // A phone with Tailscale on scans this to open openbot and pair in one step.
      if (st.code.qr)
        rows.push(
          h(
            'div',
            { class: 'set-row pair-qr-row' },
            h('div', { class: 'pair-qr', html: st.code.qr, role: 'img', 'aria-label': 'QR code for pairing a phone' }),
            h(
              'div',
              { class: 'set-row-text' },
              h('div', { class: 'set-row-title' }, 'Pair your phone'),
              h('div', { class: 'set-row-sub' }, 'Turn on Tailscale on your phone, then scan this with its camera. On an iPhone, tap Share › Add to Home Screen afterwards to open openbot like an app.'),
              h('div', { class: 'set-row-sub mono ellipsis', title: st.code.link }, st.code.link.split('#')[0])
            )
          )
        );
    }
    if (st.err) rows.push(row('Couldn’t load paired devices', btn('Try again', load), st.err));
    else if (st.list && !st.list.length) rows.push(row('No paired devices', null, 'Only this computer’s own openbot app can use it right now.'));
    for (const dv of st.list || []) {
      rows.push(
        row(
          h('span', null, dv.name, dv.current ? h('span', { class: 'chip' }, 'This device') : null),
          h(
            'button',
            {
              class: 'btn sm',
              'aria-label': `Remove ${dv.name}`,
              onclick: async () => {
                const ok = await confirmDialog({
                  title: `Remove “${dv.name}”?`,
                  body: dv.current ? 'This device will be disconnected right away. You’ll need a new pairing code to connect again.' : 'That device will lose access right away. It can only come back with a new pairing code.',
                  confirmText: 'Remove',
                  danger: true,
                });
                if (!ok) return;
                try {
                  await api('/api/devices/' + encodeURIComponent(dv.id), { method: 'DELETE' });
                  st.list = st.list.filter((x) => x.id !== dv.id);
                  rerenderSettings(true);
                } catch (e) {
                  fail(e);
                }
              },
            },
            'Remove'
          ),
          `Paired ${fmtTime(dv.created_at)} · last used ${fmtTime(dv.last_seen)}`
        )
      );
    }
    if (rows.length) wrap.append(card(rows));
    else if (!st.list) wrap.append(card(row('Loading…')));
    return wrap;
  }
  function normalizeUrl(u) {
    u = (u || '').trim();
    if (!u) return '';
    if (!/^https?:\/\//i.test(u)) u = 'http://' + u;
    return u.replace(/\/+$/, '');
  }

  // Custom MCP form (4.8)
  const mcpSlug = (n) => n.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
  function openMcpForm() {
    const st = { editor: 'form', type: 'http', name: '', url: '', headers: [['', '']], command: '', args: [''], env: [['', '']], json: '' };
    const err = h('div', { class: 'dialog-error', role: 'alert', hidden: true });
    const addBtn = h('button', { class: 'btn primary' }, 'Add plugin');
    const bodyEl = h('div');
    const kvRows = (list, kph, vph, label) => {
      const wrap = h('div', { class: 'kv-list' });
      const paint = () => {
        wrap.textContent = '';
        if (!list.length || list[list.length - 1].some((x) => x)) list.push(Array.isArray(list[0]) || !list.length ? ['', ''] : '');
        list.forEach((pair, i) => {
          if (Array.isArray(pair)) {
            const k = h('input', { class: 'field mono', placeholder: kph, value: pair[0], 'aria-label': `${label} key ${i + 1}`, spellcheck: 'false' });
            const v = h('input', { class: 'field mono', placeholder: vph, value: pair[1], 'aria-label': `${label} value ${i + 1}`, spellcheck: 'false' });
            k.addEventListener('input', () => {
              pair[0] = k.value;
              if (i === list.length - 1 && k.value) paint(), wrap.querySelectorAll('input')[i * 2].focus();
            });
            v.addEventListener('input', () => {
              pair[1] = v.value;
              if (i === list.length - 1 && v.value) paint(), wrap.querySelectorAll('input')[i * 2 + 1].focus();
            });
            wrap.append(h('div', { class: 'kv-row' }, k, v));
          } else {
            const a = h('input', { class: 'field mono', placeholder: 'Argument', value: pair, 'aria-label': `Argument ${i + 1}`, spellcheck: 'false' });
            a.addEventListener('input', () => {
              list[i] = a.value;
              if (i === list.length - 1 && a.value) paint(), wrap.querySelectorAll('input')[i].focus();
            });
            wrap.append(h('div', { class: 'kv-row one' }, a));
          }
        });
      };
      paint();
      return wrap;
    };
    const paint = () => {
      bodyEl.textContent = '';
      err.hidden = true;
      const editorSeg = h(
        'div',
        { class: 'segmented', role: 'radiogroup', 'aria-label': 'Editor' },
        [
          ['form', 'Form'],
          ['json', 'JSON'],
        ].map(([v, l]) => h('button', { class: st.editor === v ? 'on' : '', role: 'radio', 'aria-checked': st.editor === v ? 'true' : 'false', onclick: () => ((st.editor = v), paint()) }, l))
      );
      bodyEl.append(h('div', { class: 'f-inline' }, h('label', { class: 'f-label inline' }, 'Editor'), editorSeg));
      if (st.editor === 'json') {
        const ta = h('textarea', { class: 'field mono', rows: '10', placeholder: '{\n  "my-server": {\n    "type": "http",\n    "url": "https://example.com/mcp"\n  }\n}', 'aria-label': 'MCP server JSON', spellcheck: 'false' });
        ta.value = st.json;
        ta.addEventListener('input', () => (st.json = ta.value));
        bodyEl.append(h('label', { class: 'f-label' }, 'MCP server JSON'), ta);
        return;
      }
      const nameIn = h('input', { class: 'field', value: st.name, placeholder: 'My server', 'aria-label': 'Name' });
      nameIn.addEventListener('input', () => (st.name = nameIn.value));
      const typeSeg = h(
        'div',
        { class: 'segmented', role: 'radiogroup', 'aria-label': 'Type' },
        [
          ['http', 'Remote HTTPS'],
          ['stdio', 'Command'],
        ].map(([v, l]) => h('button', { class: st.type === v ? 'on' : '', role: 'radio', 'aria-checked': st.type === v ? 'true' : 'false', onclick: () => ((st.type = v), paint()) }, l))
      );
      bodyEl.append(h('label', { class: 'f-label' }, 'Name'), nameIn, h('label', { class: 'f-label' }, 'Type'), typeSeg);
      if (st.type === 'http') {
        const urlIn = h('input', { class: 'field mono', value: st.url, placeholder: 'https://example.com/mcp', 'aria-label': 'Server URL', spellcheck: 'false' });
        urlIn.addEventListener('input', () => (st.url = urlIn.value));
        bodyEl.append(h('label', { class: 'f-label' }, 'Server URL'), urlIn, h('label', { class: 'f-label' }, 'HTTP headers'), kvRows(st.headers, 'Key', 'Value', 'Header'));
      } else {
        const cmdIn = h('input', { class: 'field mono', value: st.command, placeholder: 'npx', 'aria-label': 'Command', spellcheck: 'false' });
        cmdIn.addEventListener('input', () => (st.command = cmdIn.value));
        bodyEl.append(
          h('label', { class: 'f-label' }, 'Command'),
          cmdIn,
          h('label', { class: 'f-label' }, 'Arguments'),
          kvRows(st.args, '', '', 'Argument'),
          h('label', { class: 'f-label' }, 'Environment variables'),
          kvRows(st.env, 'Key', 'Value', 'Variable'),
          h('div', { class: 'f-hint' }, 'A command runs on the computer your Bots use, in every Bot’s conversations.')
        );
      }
    };
    const fromPairs = (list) => Object.fromEntries(list.filter((p) => p[0].trim()).map((p) => [p[0].trim(), p[1]]));
    const build = () => {
      if (st.editor === 'json') {
        let v;
        try {
          v = JSON.parse(st.json);
        } catch {
          throw new Error('Enter valid JSON');
        }
        if (!v || typeof v !== 'object' || Array.isArray(v)) throw new Error('Enter valid JSON');
        const servers = v.mcpServers && typeof v.mcpServers === 'object' ? v.mcpServers : v;
        const names = Object.keys(servers);
        if (names.length !== 1) throw new Error('Add one server at a time');
        const cfg = servers[names[0]] || {};
        if (cfg.url) return { displayName: names[0], body: { type: 'http', url: String(cfg.url), headers: cfg.headers || {} } };
        if (cfg.command) return { displayName: names[0], body: { type: 'stdio', command: String(cfg.command), args: Array.isArray(cfg.args) ? cfg.args.map(String) : [], env: cfg.env || {} } };
        throw new Error('The server needs a URL or a command');
      }
      if (st.type === 'http') {
        if (!st.url.trim()) throw new Error('The server needs a URL or a command');
        return { displayName: st.name, body: { type: 'http', url: st.url.trim(), headers: fromPairs(st.headers) } };
      }
      if (!st.command.trim()) throw new Error('The server needs a URL or a command');
      return { displayName: st.name, body: { type: 'stdio', command: st.command.trim(), args: st.args.filter((a) => a !== ''), env: fromPairs(st.env) } };
    };
    addBtn.addEventListener('click', async () => {
      err.hidden = true;
      let out;
      try {
        out = build();
        const slug = mcpSlug(out.displayName || '');
        if (!slug) throw new Error('Name must contain at least one letter or number');
        if ((S.settings.mcp_servers || []).some((m) => mcpSlug(m.name) === slug) || slug === 'openbot') throw new Error(`A server named "${out.displayName.trim()}" already exists`);
        out.body.name = slug;
      } catch (e) {
        err.textContent = e.message;
        err.hidden = false;
        return;
      }
      addBtn.disabled = true;
      try {
        S.settings = await api('/api/mcp-servers', { method: 'POST', body: out.body });
        close();
        rerenderSettings(true);
      } catch (e) {
        addBtn.disabled = false;
        err.textContent = e.message;
        err.hidden = false;
      }
    });
    const box = h(
      'div',
      { class: 'dialog', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Add plugin' },
      h('button', { class: 'icon-btn dialog-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 16)),
      h('h2', { class: 'dialog-title' }, 'Add plugin'),
      h('p', { class: 'dialog-body' }, 'Connect a custom MCP server so every Bot can use its tools.'),
      bodyEl,
      err,
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn', onclick: () => close() }, 'Cancel'), addBtn)
    );
    paint();
    const close = openModal(box);
  }

  // ── Permissions (2.1) ──
  const RULE_LISTS = [
    { key: 'review_ask', title: 'Always Ask', explain: 'Stop and ask when an action matches these rules', example: 'Purchases over $50' },
    { key: 'review_allow', title: 'Allow without Asking', explain: 'Continue automatically when an action matches these rules', example: 'Emails to my team at acme.com' },
  ];
  function tabPermissions() {
    const s = S.settings;
    const ui = S.settingsUI;
    if (!s) {
      if (!S.settingsLoadErr) return h('div', { class: 'set-card skeleton', 'aria-label': 'Loading' });
      return h(
        'div',
        { class: 'set-alert bad', role: 'alert' },
        h('div', { class: 'set-alert-text' }, h('div', { class: 'set-alert-title' }, 'Couldn’t load your permissions'), h('div', null, 'Rules and Auto-review settings can’t be shown or changed until they load')),
        pillBtn('Try again', () => refreshSettingsData())
      );
    }
    const on = s.auto_review !== false;
    const out = h('div');
    const reviewCard = card(row('Auto-Review', toggleEl(on, (v) => putSettings({ auto_review: v }, 'auto-review')), 'Review openbot actions based on custom rules', '', 'auto-review'));
    out.append(h('div', { class: 'set-section' }, reviewCard));
    if (!on) {
      out.append(
        h(
          'div',
          { class: 'set-alert warn' },
          icon('alert', 16),
          h('div', { class: 'set-alert-text' }, h('div', { class: 'set-alert-title' }, 'Rules aren’t used while Auto-review is off'), h('div', null, 'openbot runs actions without checking them. Tools set to Needs approval still ask.')),
          pillBtn('Turn on', () => putSettings({ auto_review: true }, 'auto-review'))
        )
      );
    }
    const lists = h('div', { class: 'rule-lists' + (on ? '' : ' dim') });
    for (const L of RULE_LISTS) {
      const rules = s[L.key] || [];
      const ed = ui.ruleEdit && ui.ruleEdit.list === L.key ? ui.ruleEdit : null;
      const full = rules.length >= 20;
      const addBtn = pillBtn('Add Rule', () => {
        ui.ruleEdit = { list: L.key, index: -1, text: '' };
        rerenderSettings(true);
      }, { 'aria-label': `Add Rule to ${L.title}`, disabled: full || !!ui.ruleEdit || !!ui.ruleBusy });
      const head = h(
        'div',
        { class: 'set-head', id: 'set-' + (L.key === 'review_ask' ? 'always-ask-rules' : 'allow-without-asking-rules') },
        h('div', { class: 'set-head-text' }, h('h3', { class: 'set-h3' }, L.title), h('div', { class: 'set-explain' }, L.explain)),
        full && h('span', { class: 'chip' }, 'max 20 rules'),
        addBtn
      );
      const rowsEl = [];
      rules.forEach((r, i) => {
        if (ed && ed.index === i) rowsEl.push(ruleEditor(L, rules, ed));
        else
          rowsEl.push(
            h(
              'div',
              { class: 'set-row rule-row' },
              h('div', { class: 'set-row-text' }, h('div', { class: 'rule-text' }, r)),
              h(
                'div',
                { class: 'set-row-ctl' },
                h('button', { class: 'ghost-btn', 'aria-label': `Edit ${L.title} rule ${i + 1}`, 'data-tip': 'Edit', disabled: !!ui.ruleEdit, onclick: () => ((ui.ruleEdit = { list: L.key, index: i, text: r }), rerenderSettings(true)) }, icon('pencil', 15)),
                h(
                  'button',
                  {
                    class: 'ghost-btn danger',
                    'aria-label': `Delete ${L.title} rule ${i + 1}`,
                    'data-tip': 'Delete',
                    disabled: !!ui.ruleEdit,
                    onclick: () => saveRules(L.key, rules.filter((_, j) => j !== i)),
                  },
                  icon('trash', 15)
                )
              )
            )
          );
      });
      if (ed && ed.index === -1) rowsEl.push(ruleEditor(L, rules, ed));
      lists.append(h('div', { class: 'set-section' }, head, rowsEl.length ? card(rowsEl) : null));
    }
    out.append(lists);

    // Claude Code's own allow rules (added by "Always allow"), openbot-specific
    const tr = s.allow_rules || [];
    if (tr.length) {
      out.append(
        section(
          'Tool rules',
          card(tr.map((r) => row(h('span', { class: 'mono' }, r), h('button', { class: 'ghost-btn danger', 'aria-label': `Remove ${r}`, 'data-tip': 'Remove', onclick: () => putSettings({ allow_rules: tr.filter((x) => x !== r) }) }, icon('trash', 15))))),
          note('Exact tools and commands Claude Code runs without asking. “Always allow” on an approval adds one here.')
        )
      );
    }
    return out;
  }
  async function saveRules(key, list) {
    const ui = S.settingsUI;
    ui.ruleBusy = true;
    const ok = await putSettings({ [key]: list }, key === 'review_ask' ? 'always-ask-rules' : 'allow-without-asking-rules');
    ui.ruleBusy = false;
    if (ok) ui.ruleEdit = null;
    rerenderSettings(true);
  }
  function ruleEditor(L, rules, ed) {
    const ui = S.settingsUI;
    const ta = h('textarea', { class: 'field rule-input', rows: '1', maxlength: '1000', placeholder: L.example, 'aria-label': 'Applies to' });
    ta.value = ed.text || '';
    const err = h('div', { class: 'f-err', role: 'alert', hidden: true });
    const grow = () => {
      ta.style.height = 'auto';
      ta.style.height = Math.min(ta.scrollHeight + 2, 200) + 'px';
    };
    const cancel = () => {
      ui.ruleEdit = null;
      rerenderSettings(true);
    };
    const submit = () => {
      const v = ta.value.trim();
      const bad = (m) => {
        err.textContent = m;
        err.hidden = false;
        ta.focus();
      };
      if (!v) return bad('Enter a rule');
      if (rules.some((r, i) => i !== ed.index && r.trim().toLowerCase() === v.toLowerCase())) return bad('That rule is already in the list');
      if (ed.index === -1 && rules.length >= 20) return bad('You can have up to 20 rules');
      const next = rules.slice();
      if (ed.index === -1) next.push(v);
      else next[ed.index] = v;
      saveRules(L.key, next);
    };
    ta.addEventListener('input', () => {
      ed.text = ta.value;
      err.hidden = true;
      grow();
    });
    ta.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
        e.preventDefault();
        submit();
      } else if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        cancel();
      }
    });
    setTimeout(() => {
      ta.focus();
      ta.setSelectionRange(ta.value.length, ta.value.length);
      grow();
    }, 0);
    return h(
      'div',
      { class: 'set-row stack rule-editor' },
      h('label', { class: 'f-label' }, 'Applies to'),
      ta,
      err,
      h('div', { class: 'f-hint' }, 'Name the action and who or what it affects, like “Emails to dana@acme.com”'),
      h('div', { class: 'row-actions' }, btn('Cancel', cancel), btn(ed.index === -1 ? 'Add rule' : 'Save rule', submit, 'primary'))
    );
  }

  // ── Vault › Secrets: names and values checked here and again by the server ──
  // The reserved names come from the server (vault.rs), which checks them again on save.
  function secretNameProblem(n) {
    if (!n) return 'Enter an environment variable name';
    if (n.length > 128) return 'Too long. Maximum length is 128.';
    if (!/^[A-Z][A-Z0-9_]*$/.test(n)) return 'Use an uppercase environment variable name (A-Z, 0-9, _) that starts with a letter';
    const rv = (S.settings && S.settings.vault_reserved) || { names: [], prefixes: [] };
    if (rv.names.includes(n)) return `${n} is reserved: the computer, Claude Code or openbot needs it to run Bots. Pick another name.`;
    const p = rv.prefixes.find((x) => n.startsWith(x));
    if (p) return `Names starting with ${p} are reserved: Claude Code or the computer uses them. Pick another name.`;
    return '';
  }
  function secretValueProblem(v) {
    if (!v) return 'Paste the secret';
    if (v.length < 8) return 'Too short. Minimum length is 8.';
    if (new Blob([v]).size > 4096) return 'Too large. Maximum size is 4 KiB.';
    return '';
  }
  // ── Vault: logins, contact info and secrets (logins.rs, vault.rs) ──
  // "sso.ucsd.edu" from "https://www.SSO.ucsd.edu/login" (the server tidies addresses the same way).
  function siteHost(s) {
    let x = String(s || '').trim().toLowerCase();
    x = x.includes('://') ? x.split('://')[1] : x;
    x = x.split(/[/?#]/)[0];
    x = x.slice(x.lastIndexOf('@') + 1).split(':')[0].replace(/^\*\./, '').replace(/^\.+|\.+$/g, '');
    return x.replace(/^www\./, '');
  }
  const validHost = (x) => /^[a-z0-9-]+(\.[a-z0-9-]+)+$/.test(x);
  function vaultTile(kind, text) {
    if (kind === 'login') return h('span', { class: 'vault-tile', 'aria-hidden': 'true' }, (siteHost(text)[0] || '?').toUpperCase());
    return h('span', { class: 'vault-tile icon', 'aria-hidden': 'true' }, icon(kind === 'contact' ? 'person' : 'key', 16));
  }
  // Text with the search words highlighted.
  function marked(text, q) {
    text = String(text || '');
    if (!q) return text;
    const i = text.toLowerCase().indexOf(q);
    if (i < 0) return text;
    return h('span', null, text.slice(0, i), h('mark', null, text.slice(i, i + q.length)), text.slice(i + q.length));
  }
  const contactName = (c) => c.label || c.full_name || c.emails[0] || c.phones[0] || 'Contact info';
  const loginTitle = (l) => l.sites[0] || 'Login';

  function tabVault() {
    const s = S.settings;
    if (!s) return notConnected();
    const ui = S.settingsUI;
    const v = (ui.vault = ui.vault || { tab: 'all', q: '' });
    const logins = s.logins || [];
    const contacts = s.contacts || [];
    const secrets = s.vault || [];
    const q = v.q.trim().toLowerCase();
    const has = (...xs) => !q || xs.some((x) => String(x || '').toLowerCase().includes(q));
    const fl = logins.filter((l) => has(...l.sites, l.username, l.hints));
    const fc = contacts.filter((c) => has(c.label, c.full_name, c.city, ...c.emails, ...c.phones));
    const fs = secrets.filter((x) => has(x.name, x.description));
    const out = h('div');

    const newBtn = h('button', { class: 'btn primary pill vault-new', 'aria-haspopup': 'menu' }, 'New item', icon('chevronDown', 14));
    newBtn.addEventListener('click', () => {
      const r = newBtn.getBoundingClientRect();
      showMenu(
        [
          { label: 'Login', icon: 'key', action: () => loginDialog(null) },
          { label: 'Contact info', icon: 'person', action: () => contactDialog(null) },
          { label: 'Secret (for commands)', icon: 'terminal', action: () => ((v.tab = 'secrets'), (ui.vaultForm = { mode: 'add' }), rerenderSettings(true)) },
          '-',
          { label: 'Import', icon: 'file', action: () => importDialog() },
        ],
        r.left,
        r.bottom + 6
      );
    });
    out.append(
      h(
        'div',
        { class: 'set-head' },
        h('div', { class: 'set-head-text' }, h('div', { class: 'set-explain' }, 'Logins your Bots can sign in with, contact info they can fill forms with, and secrets for commands.')),
        newBtn
      )
    );

    // Tabs and search.
    const tabs = [
      ['all', 'All', logins.length + contacts.length + secrets.length],
      ['logins', 'Logins', logins.length],
      ['contacts', 'Contact info', contacts.length],
      ['secrets', 'Secrets', secrets.length],
    ];
    const search = h('input', { class: 'field vault-search', type: 'search', placeholder: 'Search Vault', value: v.q, 'aria-label': 'Search Vault' });
    search.addEventListener('input', () => {
      v.q = search.value;
      rerenderSettings(true);
      const el = document.querySelector('.vault-search');
      if (el) {
        el.focus();
        el.setSelectionRange(el.value.length, el.value.length);
      }
    });
    search.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && search.value) {
        e.stopPropagation();
        v.q = '';
        rerenderSettings(true);
      }
    });
    out.append(
      h(
        'div',
        { class: 'vault-bar' },
        h(
          'div',
          { class: 'segmented vault-tabs', role: 'tablist', 'aria-label': 'Item type' },
          tabs.map(([id, label, n]) =>
            h('button', { class: 'seg' + (v.tab === id ? ' on' : ''), role: 'tab', 'aria-selected': v.tab === id ? 'true' : 'false', onclick: () => ((v.tab = id), rerenderSettings(true)) }, label, h('span', { class: 'vault-count' }, String(n)))
          )
        ),
        search
      )
    );

    const loginRows = () =>
      fl.map((l) =>
        h(
          'button',
          { class: 'vault-row', onclick: () => loginDialog(l), 'aria-label': `View login for ${loginTitle(l)}${l.username ? ` (${l.username})` : ''}` },
          vaultTile('login', loginTitle(l)),
          h('span', { class: 'vault-row-text' }, h('span', { class: 'vault-row-title' }, marked(loginTitle(l), q), l.sites.length > 1 ? h('span', { class: 'vault-more' }, ` +${l.sites.length - 1}`) : null), h('span', { class: 'vault-row-sub' }, l.username ? marked(l.username, q) : l.hints ? `Hint: ${l.hints}` : 'No username'), l.locked ? h('span', { class: 'chip warn' }, 'Locked') : null),
          icon('chevronRight', 16)
        )
      );
    const contactRows = () =>
      fc.map((c) =>
        h(
          'button',
          { class: 'vault-row', onclick: () => contactDialog(c), 'aria-label': `Edit ${contactName(c)}` },
          vaultTile('contact'),
          h('span', { class: 'vault-row-text' }, h('span', { class: 'vault-row-title' }, marked(contactName(c), q)), h('span', { class: 'vault-row-sub' }, [c.full_name !== contactName(c) ? c.full_name : '', c.emails[0], c.phones[0], c.city].filter(Boolean).join(' · ') || 'Contact info')),
          icon('chevronRight', 16)
        )
      );
    const secretRows = () =>
      fs.map((x) =>
        h(
          'button',
          { class: 'vault-row', onclick: () => ((v.tab = 'secrets'), (ui.vaultForm = { mode: 'replace', name: x.name }), rerenderSettings(true)), 'aria-label': `Edit ${x.name}` },
          vaultTile('secret'),
          h('span', { class: 'vault-row-text' }, h('span', { class: 'vault-row-title mono' }, marked(x.name, q)), h('span', { class: 'vault-row-sub' }, x.description || x.hint || 'Secret')),
          icon('chevronRight', 16)
        )
      );
    const EMPTY = {
      all: 'Nothing saved yet. Add a login, contact info or a secret with New item.',
      logins: 'No logins yet. Add one with New item, or save one from a secure form in a chat.',
      contacts: 'No contact info yet. Add some with New item.',
      secrets: 'No secrets yet. Add one with New item.',
    };
    const listCard = (rows, empty) => card(rows.length ? h('div', { class: 'vault-list' }, rows) : h('div', { class: 'set-empty' }, q ? `No matches for “${v.q.trim()}”` : empty));
    if (v.tab === 'all') {
      const groups = [
        ['Logins', loginRows()],
        ['Contact info', contactRows()],
        ['Secrets', secretRows()],
      ].filter(([, rows]) => rows.length);
      if (!groups.length) out.append(listCard([], EMPTY.all));
      for (const [label, rows] of groups) out.append(section(label, listCard(rows, '')));
    } else if (v.tab === 'logins') out.append(listCard(loginRows(), EMPTY.logins));
    else if (v.tab === 'contacts') out.append(listCard(contactRows(), EMPTY.contacts));
    else out.append(secretsCard(fs));

    out.append(
      note(
        v.tab === 'secrets'
          ? 'Secrets are environment variables for your Bots’ shell commands. Bots see the names and what they are for, never the values. '
          : 'Bots never see saved passwords: openbot types them into the login page on the Bot’s screen, and only on the websites you saved them for. Bots can see usernames and contact info so they can type them in. ',
        h('span', { class: 'secret-protection' }, (s.vault_protection || 'Not encrypted') + '.')
      )
    );
    return out;
  }

  // Secrets tab: environment variables for shell commands, edited in place.
  function secretsCard(vault) {
    const ui = S.settingsUI;
    const form = ui.vaultForm;
    const rows = vault.map((v) =>
      form && form.mode === 'replace' && form.name === v.name
        ? secretForm(form, v)
        : row(
            h(
              'span',
              { class: 'secret-name' },
              icon('key', 15),
              h('span', { class: 'mono' }, v.name),
              v.hint && h('span', { class: 'secret-hint mono' }, v.hint),
              v.locked && h('span', { class: 'chip warn' }, 'Locked'),
              v.name_problem && h('span', { class: 'chip warn' }, 'Not used')
            ),
            h(
              'div',
              { class: 'secret-ctl' },
              pillBtn('Edit', () => ((ui.vaultForm = { mode: 'replace', name: v.name }), rerenderSettings(true)), { disabled: !!form, 'aria-label': `Edit ${v.name}` }),
              pillBtn('Remove', () => removeSecret(v.name), { disabled: !!form, 'aria-label': `Remove ${v.name}` })
            ),
            h(
              'span',
              null,
              v.description ? h('span', { class: 'secret-desc' }, v.description) : null,
              h('span', { class: 'secret-access' }, scopeText(v.bots, 'Edit')),
              v.locked && h('span', { class: 'secret-problem' }, 'This value can’t be unlocked on this computer, so no Bot gets it. Use Edit to enter it again.'),
              v.name_problem && h('span', { class: 'secret-problem' }, v.name_problem + ' Bots don’t get it until you remove it and add it under another name.')
            )
          )
    );
    if (form && form.mode === 'add') rows.push(secretForm(form, null));
    return h(
      'div',
      { class: 'set-section' },
      h('div', { class: 'vault-sub-head' }, pillBtn('Add secret', () => ((ui.vaultForm = { mode: 'add' }), rerenderSettings(true)), { disabled: !!form })),
      rows.length ? card(rows) : card(h('div', { class: 'set-empty' }, 'No secrets yet'))
    );
  }

  // A dialog whose content is redrawn by draw(); returns {box, close, redraw}.
  function vaultModal(cls, draw) {
    const box = h('div', { class: 'dialog vault-dlg ' + cls, role: 'dialog', 'aria-modal': 'true' });
    const close = openModal(box);
    const redraw = () => box.replaceChildren(...[].concat(draw()).filter(Boolean));
    redraw();
    return { box, close, redraw };
  }
  const dlgClose = (close) => h('button', { class: 'icon-btn dialog-close', 'aria-label': 'Close dialog', onclick: () => close() }, icon('x', 16));
  async function vaultSave(st, path, method, body, after) {
    st.busy = true;
    st.err = '';
    st.redraw();
    try {
      S.settings = await api(path, { method, body });
      after && after();
      rerenderSettings(true);
    } catch (e) {
      st.busy = false;
      st.err = e.message || 'Couldn’t save. Try again.';
      st.redraw();
    }
  }

  // ── Login dialog: view, edit, new ──
  function loginDialog(login, startEdit) {
    const st = {
      mode: login && !startEdit ? 'view' : 'edit',
      sites: login ? login.sites.slice() : [''],
      username: login ? login.username : '',
      password: '',
      hints: login ? login.hints : '',
      bots: login && login.bots ? login.bots.slice() : null,
      err: '',
      busy: false,
    };
    const start = JSON.stringify([st.sites, st.username, st.hints, st.bots]);
    const changed = () => st.password !== '' || JSON.stringify([st.sites.filter((x) => x.trim()), st.username, st.hints, st.bots]) !== start;
    const firstHost = () => siteHost(st.sites.find((x) => validHost(siteHost(x))) || '');
    let m;
    const remove = async () => {
      const ok = await confirmDialog({ title: 'Remove this login?', body: `Bots can no longer sign in to ${loginTitle(login)} with it.`, confirmText: 'Remove', danger: true });
      if (ok) vaultSave(st, '/api/vault/logins/' + encodeURIComponent(login.id), 'DELETE', undefined, () => m.close());
    };
    const view = () => {
      const r = (label, ...kids) => h('div', { class: 'vault-field' }, h('div', { class: 'vault-field-label' }, label), h('div', { class: 'vault-field-value' }, ...kids));
      return [
        dlgClose(() => m.close()),
        h('div', { class: 'vault-dlg-head' }, vaultTile('login', loginTitle(login)), h('h2', { class: 'dialog-title' }, loginTitle(login))),
        r('Website', ...login.sites.map((x) => h('div', { class: 'vault-site' }, h('span', null, x), h('a', { class: 'icon-btn', href: 'https://' + x, target: '_blank', rel: 'noopener noreferrer', 'aria-label': `Open ${x}` }, icon('external', 14))))),
        login.username ? r('Username', h('span', { class: 'mono' }, login.username)) : null,
        r('Password', login.locked ? h('span', { class: 'secret-problem' }, 'Can’t be unlocked on this computer. Use Edit to enter it again.') : h('span', { class: 'vault-dots', 'aria-label': 'Saved password, hidden' }, '••••••••••')),
        login.hints ? r('Hints', login.hints) : null,
        r('Bots', scopeText(login.bots, 'Edit')),
        st.err ? h('div', { class: 'dialog-error', role: 'alert' }, st.err) : null,
        h('div', { class: 'dialog-actions split' }, h('button', { class: 'btn danger', disabled: st.busy, onclick: remove }, 'Remove'), h('span', { class: 'grow' }), btn('Edit', () => ((st.mode = 'edit'), m.redraw())), btn('Done', () => m.close(), 'primary')),
      ];
    };
    const edit = () => {
      const save = h('button', { class: 'btn primary', disabled: st.busy || !firstHost() || !changed() || (!login && !st.password) }, 'Save login');
      const sync = () => (save.disabled = st.busy || !firstHost() || !changed() || (!login && !st.password));
      const siteRows = st.sites.map((val, i) => {
        const inp = h('input', { class: 'field', value: val, placeholder: 'example.com', spellcheck: 'false', autocomplete: 'off', 'aria-label': i ? `Website ${i + 1}` : 'Website' });
        inp.addEventListener('input', () => {
          st.sites[i] = inp.value;
          sync();
          hint.textContent = firstHost() ? `Fills on ${firstHost()} and its subdomains` : '';
        });
        // Pasting several addresses separated by spaces makes one row each.
        inp.addEventListener('paste', (e) => {
          const t = (e.clipboardData && e.clipboardData.getData('text')) || '';
          const parts = t.split(/\s+/).filter(Boolean);
          if (parts.length < 2) return;
          e.preventDefault();
          st.sites.splice(i, 1, ...parts);
          st.sites = st.sites.slice(0, 10);
          m.redraw();
        });
        return h('div', { class: 'vault-site-row' }, inp, i ? h('button', { class: 'icon-btn', 'aria-label': `Remove website ${i + 1}`, onclick: () => (st.sites.splice(i, 1), m.redraw()) }, icon('x', 14)) : null);
      });
      const hint = h('div', { class: 'f-hint' }, firstHost() ? `Fills on ${firstHost()} and its subdomains` : '');
      const user = h('input', { class: 'field', value: st.username, placeholder: 'name@example.com', autocomplete: 'off', spellcheck: 'false', 'aria-label': 'Username or email' });
      user.addEventListener('input', () => ((st.username = user.value), sync()));
      const pw = h('input', { class: 'field mono', type: 'password', value: st.password, placeholder: login && !login.locked ? 'Leave empty to keep the saved password' : '', autocomplete: 'new-password', 'aria-label': 'Password' });
      pw.addEventListener('input', () => ((st.password = pw.value), sync()));
      const hints = h('textarea', { class: 'field vault-hints', maxlength: '2000', placeholder: 'Use this for the student portal, not the staff one', 'aria-label': 'Hints' }, st.hints);
      hints.addEventListener('input', () => ((st.hints = hints.value), sync()));
      save.addEventListener('click', () => {
        const body = { sites: st.sites.filter((x) => x.trim()), username: st.username, password: st.password, hints: st.hints, bots: st.bots };
        vaultSave(st, login ? '/api/vault/logins/' + encodeURIComponent(login.id) : '/api/vault/logins', 'POST', body, () => m.close());
      });
      setTimeout(() => (login ? pw : m.box.querySelector('input')).focus(), 30);
      return [
        dlgClose(() => m.close()),
        h('h2', { class: 'dialog-title' }, login ? `Edit login for ${loginTitle(login)}` : 'New login'),
        h('label', { class: 'f-label' }, 'Website'),
        ...siteRows,
        st.sites.length < 10 ? h('button', { class: 'btn link vault-add', onclick: () => (st.sites.push(''), m.redraw()) }, icon('plus', 14), 'Add website') : null,
        hint,
        h('label', { class: 'f-label' }, 'Username or email'),
        user,
        h('label', { class: 'f-label' }, login && login.locked ? 'Password (enter it again)' : 'Password'),
        pw,
        h('label', { class: 'f-label' }, 'Hints'),
        hints,
        h('div', { class: 'f-hint' }, 'Tell the Bot when to use this login.'),
        h('label', { class: 'f-label' }, 'Which Bots can use it'),
        botScopePicker(st, sync),
        !login && firstHost() ? h('div', { class: 'f-hint vault-note' }, `Bots can use this login to sign in for you on ${firstHost()}.`) : null,
        st.err ? h('div', { class: 'dialog-error', role: 'alert' }, st.err) : null,
        h('div', { class: 'dialog-actions split' }, login ? h('button', { class: 'btn danger', disabled: st.busy, onclick: remove }, 'Remove') : null, h('span', { class: 'grow' }), btn('Cancel', () => (login && st.mode === 'edit' && !startEdit ? ((st.mode = 'view'), m.redraw()) : m.close())), save),
      ];
    };
    m = vaultModal('', () => (st.mode === 'view' ? view() : edit()));
    st.redraw = m.redraw;
  }

  // ── Contact info dialog ──
  function contactDialog(c) {
    const st = Object.assign({ full_name: '', address1: '', address2: '', city: '', region: '', postal_code: '', country: '', label: '', hints: '' }, c || {});
    st.phones = c ? c.phones.slice() : [''];
    st.emails = c ? c.emails.slice() : [''];
    st.bots = c && c.bots ? c.bots.slice() : null;
    st.err = '';
    st.busy = false;
    let m;
    const text = (key, label, attrs = {}) => {
      const inp = h('input', Object.assign({ class: 'field', value: st[key], 'aria-label': label, autocomplete: 'off' }, attrs));
      inp.addEventListener('input', () => (st[key] = inp.value));
      return h('div', { class: 'vault-cell' }, h('label', { class: 'f-label' }, label), inp);
    };
    // Phone and email lists: the first one is the primary.
    const list = (key, label, check) => {
      const rows = st[key].map((val, i) => {
        const inp = h('input', { class: 'field', value: val, type: key === 'emails' ? 'email' : 'tel', 'aria-label': i ? `${label} ${i + 1}` : label, autocomplete: 'off' });
        const err = h('div', { class: 'f-err', role: 'alert', hidden: true });
        inp.addEventListener('input', () => ((st[key][i] = inp.value), (err.hidden = true)));
        inp.addEventListener('blur', () => {
          const p = check(inp.value.trim(), i);
          err.textContent = p;
          err.hidden = !p;
        });
        return h(
          'div',
          { class: 'vault-list-row' },
          h('label', { class: 'f-label' }, i ? `${label} ${i + 1}` : label),
          h('div', { class: 'vault-site-row' }, inp, i ? h('button', { class: 'btn link', onclick: () => (st[key].unshift(...st[key].splice(i, 1)), m.redraw()) }, 'Make primary') : null, st[key].length > 1 ? h('button', { class: 'icon-btn', 'aria-label': `Remove ${label.toLowerCase()} ${i + 1}`, onclick: () => (st[key].splice(i, 1), m.redraw()) }, icon('x', 14)) : null),
          err
        );
      });
      return [...rows, st[key].length < 10 ? h('button', { class: 'btn link vault-add', onclick: () => (st[key].push(''), m.redraw()) }, icon('plus', 14), `Add ${label.toLowerCase()}`) : null];
    };
    const phoneProblem = (v, i) => {
      if (!v) return '';
      const digits = v.replace(/\D/g, '').length;
      if (digits < 7 || digits > 15 || !/^[0-9 +\-().]+$/.test(v)) return 'Enter a valid phone number';
      return st.phones.slice(0, i).some((x) => x.trim() === v) ? 'Already listed above' : '';
    };
    const emailProblem = (v, i) => {
      if (!v) return '';
      if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(v)) return 'Enter a valid email';
      return st.emails.slice(0, i).some((x) => x.trim().toLowerCase() === v.toLowerCase()) ? 'Already listed above' : '';
    };
    const remove = async () => {
      const ok = await confirmDialog({ title: `Remove “${contactName(c)}”?`, body: 'Bots can no longer use this contact info.', confirmText: 'Remove', danger: true });
      if (ok) vaultSave(st, '/api/vault/contacts/' + encodeURIComponent(c.id), 'DELETE', undefined, () => m.close());
    };
    const save = () => {
      const bad = st.phones.some((x, i) => phoneProblem(x.trim(), i)) || st.emails.some((x, i) => emailProblem(x.trim(), i));
      if (bad) {
        st.err = 'Fix the phone numbers and emails marked above.';
        return m.redraw();
      }
      const body = Object.assign({}, st, { phones: st.phones.map((x) => x.trim()).filter(Boolean), emails: st.emails.map((x) => x.trim()).filter(Boolean) });
      delete body.err;
      delete body.busy;
      delete body.redraw;
      vaultSave(st, c ? '/api/vault/contacts/' + encodeURIComponent(c.id) : '/api/vault/contacts', 'POST', body, () => m.close());
    };
    m = vaultModal('wide', () => [
      dlgClose(() => m.close()),
      h('h2', { class: 'dialog-title' }, c ? `Edit ${contactName(c)}` : 'New contact info'),
      text('full_name', 'Full name', { autocomplete: 'off' }),
      text('address1', 'Address'),
      text('address2', 'Address line 2'),
      h('div', { class: 'vault-pair' }, text('city', 'City'), text('region', 'State or region')),
      h('div', { class: 'vault-pair' }, text('postal_code', 'ZIP or postal code'), text('country', 'Country')),
      ...list('phones', 'Phone', phoneProblem),
      ...list('emails', 'Email', emailProblem),
      text('label', 'Label', { placeholder: 'Home, Work…' }),
      h('label', { class: 'f-label' }, 'Hints'),
      (() => {
        const t = h('textarea', { class: 'field vault-hints', maxlength: '2000', 'aria-label': 'Hints', placeholder: 'Use the work address for deliveries' }, st.hints);
        t.addEventListener('input', () => (st.hints = t.value));
        return t;
      })(),
      h('label', { class: 'f-label' }, 'Which Bots can use it'),
      botScopePicker(st),
      st.err ? h('div', { class: 'dialog-error', role: 'alert' }, st.err) : null,
      h('div', { class: 'dialog-actions split' }, c ? h('button', { class: 'btn danger', disabled: st.busy, onclick: remove }, 'Remove') : null, h('span', { class: 'grow' }), btn('Cancel', () => m.close()), h('button', { class: 'btn primary', disabled: st.busy, onclick: save }, 'Save contact info')),
    ]);
    st.redraw = m.redraw;
  }

  // ── Import logins from a password manager's CSV export ──
  // A CSV table (quoted fields may hold commas, quotes and line breaks).
  function parseCsv(text) {
    const rows = [];
    let row = [];
    let cell = '';
    let quoted = false;
    for (let i = 0; i < text.length; i++) {
      const ch = text[i];
      if (quoted) {
        if (ch === '"' && text[i + 1] === '"') (cell += '"'), i++;
        else if (ch === '"') quoted = false;
        else cell += ch;
      } else if (ch === '"') quoted = true;
      else if (ch === ',') row.push(cell), (cell = '');
      else if (ch === '\n' || ch === '\r') {
        if (ch === '\r' && text[i + 1] === '\n') i++;
        row.push(cell);
        rows.push(row);
        row = [];
        cell = '';
      } else cell += ch;
    }
    if (cell || row.length) row.push(cell), rows.push(row);
    return rows;
  }
  // Rows {url, username, password, name} from an export by 1Password, Chrome, Edge, Safari / Apple
  // Passwords, Firefox, Bitwarden or LastPass. null = not a password export.
  function exportRows(text) {
    const table = parseCsv(text.replace(/^﻿/, ''));
    if (table.length < 1) return null;
    const head = table[0].map((x) => x.trim().toLowerCase());
    const col = (...names) => names.map((n) => head.indexOf(n)).find((i) => i >= 0) ?? -1;
    const url = col('url', 'login_uri', 'website', 'login url');
    const user = col('username', 'login_username', 'user name', 'email');
    const pass = col('password', 'login_password');
    const name = col('name', 'title');
    const type = col('type');
    if (url < 0 || pass < 0) return null;
    return table
      .slice(1)
      .filter((r) => r.some((x) => x.trim()))
      .filter((r) => type < 0 || !r[type] || r[type].toLowerCase() === 'login')
      .map((r) => ({ url: r[url] || '', username: user >= 0 ? r[user] || '' : '', password: r[pass] || '', name: name >= 0 ? r[name] || '' : '' }));
  }
  function importDialog() {
    const st = { step: 'pick', rows: null, preview: null, result: null, err: '', busy: false };
    let m;
    const file = h('input', { type: 'file', accept: '.csv,text/csv', hidden: true });
    file.addEventListener('change', async () => {
      const f = file.files && file.files[0];
      file.value = '';
      if (!f) return;
      st.err = '';
      const rows = exportRows(await f.text());
      if (!rows) {
        st.err = 'This doesn’t look like a password export. Choose a CSV exported from a supported app.';
        return m.redraw();
      }
      st.busy = true;
      m.redraw();
      try {
        st.preview = await api('/api/vault/import', { method: 'POST', body: { rows, apply: false } });
        st.rows = rows;
        st.step = 'preview';
      } catch (e) {
        st.err = e.message;
      }
      st.busy = false;
      m.redraw();
    });
    const doImport = async () => {
      st.busy = true;
      m.redraw();
      try {
        const r = await api('/api/vault/import', { method: 'POST', body: { rows: st.rows, apply: true } });
        if (r.settings) S.settings = r.settings;
        st.result = r;
        st.rows = null; // the passwords are on the computer now; drop this copy
        st.step = 'done';
        rerenderSettings(true);
      } catch (e) {
        st.err = e.message;
      }
      st.busy = false;
      m.redraw();
    };
    const LABEL = { ready: 'Ready', duplicate: 'Already in your vault', full: 'Vault full' };
    const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
    m = vaultModal('wide', () => {
      const head = [dlgClose(() => m.close()), h('h2', { class: 'dialog-title' }, 'Import logins')];
      const err = st.err ? h('div', { class: 'dialog-error', role: 'alert' }, st.err) : null;
      if (st.step === 'pick')
        return [
          ...head,
          h('p', { class: 'dialog-body' }, 'Import logins from a CSV file exported by 1Password, Chrome, Edge, Safari, Apple Passwords, Firefox, Bitwarden, or LastPass.'),
          file,
          err,
          h('div', { class: 'dialog-actions' }, btn('Cancel', () => m.close()), h('button', { class: 'btn primary', disabled: st.busy, onclick: () => file.click() }, st.busy ? 'Reading…' : 'Choose file')),
        ];
      if (st.step === 'preview') {
        const p = st.preview;
        const chips = [p.ready && `${p.ready} ready`, p.duplicates && `${p.duplicates} already in your vault`, p.errors && plural(p.errors, 'with errors', 'with errors'), p.skipped_full && `${p.skipped_full} skipped`].filter(Boolean);
        const items = p.items.slice(0, 300);
        return [
          ...head,
          h('div', { class: 'vault-chips' }, chips.map((t) => h('span', { class: 'chip' }, t))),
          h(
            'div',
            { class: 'vault-import-list' },
            items.map((it) => h('div', { class: 'vault-import-row' + (it.status === 'ready' ? '' : ' dim') }, vaultTile('login', it.title), h('span', { class: 'vault-row-text' }, h('span', { class: 'vault-row-title' }, it.title || 'No website'), h('span', { class: 'vault-row-sub' }, it.username || '')), h('span', { class: 'chip' + (it.status === 'error' ? ' warn' : '') }, it.error || LABEL[it.status])))
          ),
          p.items.length > items.length ? h('div', { class: 'f-hint' }, `…and ${p.items.length - items.length} more`) : null,
          err,
          h('div', { class: 'dialog-actions' }, btn('Cancel', () => m.close()), h('button', { class: 'btn primary', disabled: st.busy || !p.ready, onclick: doImport }, st.busy ? 'Importing…' : `Import ${plural(p.ready, 'login', 'logins')}`)),
        ];
      }
      const r = st.result;
      return [
        ...head,
        h('p', { class: 'dialog-body' }, `Imported ${plural(r.ready, 'login', 'logins')}.`),
        r.duplicates ? h('p', { class: 'dialog-body' }, `Skipped ${plural(r.duplicates, 'item', 'items')} already in your vault.`) : null,
        r.skipped_full ? h('p', { class: 'dialog-body' }, `Your vault is full. ${plural(r.skipped_full, 'item', 'items')} couldn’t be imported.`) : null,
        h('p', { class: 'dialog-body vault-warn' }, 'Delete the exported file now: it has your passwords in plain text.'),
        h('div', { class: 'dialog-actions' }, btn('Done', () => m.close(), 'primary')),
      ];
    });
  }
  async function removeSecret(name) {
    const ok = await confirmDialog({ title: `Remove ${name}?`, body: 'Bots can no longer use this secret.', confirmText: 'Remove', danger: true });
    if (!ok) return;
    try {
      S.settings = await api('/api/vault/' + encodeURIComponent(name), { method: 'DELETE' });
    } catch (e) {
      fail(e);
    }
    rerenderSettings(true);
  }
  function secretForm(form, existing) {
    const ui = S.settingsUI;
    const nameIn = h('input', { class: 'field mono', value: existing ? existing.name : form.nameV || '', placeholder: 'GITHUB_TOKEN', maxlength: '128', spellcheck: 'false', autocomplete: 'off', 'aria-label': 'Environment variable name', disabled: !!existing });
    const descIn = h('input', { class: 'field', value: existing ? existing.description || '' : form.descV || '', placeholder: 'GitHub access for opening pull requests', maxlength: '300', 'aria-label': 'What it is for (shown to the Bot)' });
    const keepOk = existing && !existing.locked; // an empty value keeps the saved one
    const valIn = h('input', { class: 'field mono', type: 'password', placeholder: keepOk ? 'Leave empty to keep the current value' : '', autocomplete: 'new-password', 'aria-label': 'Value' });
    // Which Bots can use it: null = all, else a list of Bot ids.
    if (!('bots' in form)) form.bots = existing && existing.bots ? existing.bots.slice() : null;
    const botsErr = h('div', { class: 'f-err', role: 'alert', hidden: true });
    const picker = botScopePicker(form, () => (botsErr.hidden = true));
    const nameErr = h('div', { class: 'f-err', role: 'alert', hidden: true });
    const valErr = h('div', { class: 'f-err', role: 'alert', hidden: true });
    nameIn.addEventListener('input', () => {
      const p = nameIn.selectionStart;
      nameIn.value = nameIn.value.toUpperCase().replace(/[\s-]/g, '_');
      nameIn.setSelectionRange(p, p);
      nameErr.hidden = true;
    });
    valIn.addEventListener('input', () => (valErr.hidden = true));
    const save = async (e) => {
      const name = nameIn.value.trim();
      const value = valIn.value;
      const np = existing ? '' : secretNameProblem(name) || (S.settings.vault || []).some((x) => x.name === name) ? secretNameProblem(name) || `${name} already exists. Use Replace to change it.` : '';
      const vp = keepOk && !value ? '' : secretValueProblem(value);
      const bp = form.bots !== null && !form.bots.length ? 'Pick at least one Bot, or choose All Bots' : '';
      nameErr.textContent = np;
      nameErr.hidden = !np;
      valErr.textContent = vp;
      valErr.hidden = !vp;
      botsErr.textContent = bp;
      botsErr.hidden = !bp;
      if (np) return nameIn.focus();
      if (vp) return valIn.focus();
      if (bp) return;
      e.currentTarget.disabled = true;
      try {
        S.settings = await api('/api/vault', { method: 'POST', body: { name, value, description: descIn.value.trim(), bots: form.bots } });
        ui.vaultForm = null;
      } catch (x) {
        e.currentTarget.disabled = false;
        valErr.textContent = x.message;
        valErr.hidden = false;
        return;
      }
      rerenderSettings(true);
    };
    const cancel = () => {
      ui.vaultForm = null;
      rerenderSettings(true);
    };
    for (const el of [nameIn, descIn, valIn])
      el.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') save({ currentTarget: e.target.closest('.secret-form').querySelector('.btn.primary') });
        if (e.key === 'Escape') {
          e.stopPropagation();
          cancel();
        }
      });
    setTimeout(() => (existing ? valIn : nameIn).focus(), 0);
    return h(
      'div',
      { class: 'set-row stack secret-form' },
      h('label', { class: 'f-label' }, 'Environment variable name'),
      nameIn,
      nameErr,
      h('label', { class: 'f-label' }, 'What it is for (shown to the Bot)'),
      descIn,
      h('label', { class: 'f-label' }, existing && existing.locked ? 'Value (enter it again)' : 'Value'),
      valIn,
      valErr,
      h('label', { class: 'f-label' }, 'Which Bots can use it'),
      picker,
      botsErr,
      h('div', { class: 'row-actions' }, btn('Cancel', cancel), btn(existing ? 'Save' : 'Add secret', save, 'primary'))
    );
  }

  // ── Connections: Google and Microsoft accounts (accounts.rs) ──
  const PROVIDER_LOGO = {
    google:
      '<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path fill="#4285F4" d="M22.6 12.3c0-.8-.1-1.5-.2-2.3H12v4.3h5.9a5 5 0 01-2.2 3.3v2.8h3.6c2.1-1.9 3.3-4.8 3.3-8.1z"/><path fill="#34A853" d="M12 23c3 0 5.5-1 7.3-2.7l-3.6-2.8c-1 .7-2.2 1.1-3.7 1.1-2.9 0-5.3-1.9-6.2-4.5H2.1v2.9A11 11 0 0012 23z"/><path fill="#FBBC05" d="M5.8 14.1a6.6 6.6 0 010-4.2V7H2.1a11 11 0 000 10z"/><path fill="#EA4335" d="M12 5.4c1.6 0 3.1.6 4.2 1.7l3.2-3.2A11 11 0 002.1 7l3.7 2.9C6.7 7.3 9.1 5.4 12 5.4z"/></svg>',
    github:
      '<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path fill="currentColor" d="M12 1.5a10.5 10.5 0 00-3.3 20.5c.5.1.7-.2.7-.5v-1.8c-2.9.6-3.5-1.4-3.5-1.4-.5-1.2-1.2-1.5-1.2-1.5-.9-.6.1-.6.1-.6 1 .1 1.6 1.1 1.6 1.1.9 1.6 2.5 1.1 3.1.9.1-.7.4-1.1.7-1.4-2.3-.3-4.8-1.2-4.8-5.2 0-1.1.4-2.1 1.1-2.8-.1-.3-.5-1.3.1-2.8 0 0 .9-.3 2.9 1.1a10 10 0 015.2 0c2-1.4 2.9-1.1 2.9-1.1.6 1.5.2 2.5.1 2.8.7.8 1.1 1.7 1.1 2.8 0 4-2.5 4.9-4.8 5.2.4.3.7 1 .7 1.9v2.9c0 .3.2.6.7.5A10.5 10.5 0 0012 1.5z"/></svg>',
    microsoft:
      '<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path fill="#F25022" d="M2 2h9.5v9.5H2z"/><path fill="#7FBA00" d="M12.5 2H22v9.5h-9.5z"/><path fill="#00A4EF" d="M2 12.5h9.5V22H2z"/><path fill="#FFB900" d="M12.5 12.5H22V22h-9.5z"/></svg>',
  };
  const PROVIDER_NAME = { google: 'Google', microsoft: 'Microsoft', github: 'GitHub' };
  function providerLogo(p) {
    const el = h('span', { class: 'acct-logo' });
    el.innerHTML = PROVIDER_LOGO[p] || '';
    return el;
  }
  // Where the browser comes back after signing in: this device's own openbot (the desktop app's
  // local server), which passes it on to the computer your Bots run on.
  function signInReturn() {
    const local = /^(127\.0\.0\.1|localhost|\[::1\])$/.test(location.hostname);
    return local ? location.origin + '/oauth/callback' : null;
  }
  async function connectAccount(provider, reconnect) {
    const ui = S.settingsUI;
    const back = signInReturn() || 'http://127.0.0.1:1/oauth/callback';
    try {
      const r = await api('/api/accounts/connect', { method: 'POST', body: { provider, redirect: back, reconnect: reconnect || null } });
      ui.connect = { provider, reconnect: reconnect || null, url: r.url, paste: '', manual: !signInReturn(), err: '' };
      openExternal(r.url);
    } catch (e) {
      fail(e);
      return;
    }
    rerenderSettings(true);
  }
  // "All Bots can use it", "Only Food Bot and Mail Bot can use it". button = where to change it.
  function scopeText(bots, button) {
    if (!bots) return 'All Bots can use it';
    const names = bots.map((id) => S.bots.get(id)).filter(Boolean).map((b) => b.name);
    if (!names.length) return `No Bots can use it. Use ${button} to pick some.`;
    const list = names.length === 1 ? names[0] : names.slice(0, -1).join(', ') + ' and ' + names[names.length - 1];
    return `Only ${list} can use it`;
  }
  // Shared "All Bots / Only the Bots I pick" picker. st.bots: null = all, else ids.
  function botScopePicker(st, onChange) {
    const wrap = h('div', { class: 'scope-pick' });
    const list = h('div', { class: 'secret-bots', role: 'group', 'aria-label': 'Bots that can use it' });
    const draw = () => {
      list.hidden = st.bots === null;
      list.replaceChildren(
        ...orderedBots().filter((b) => !isGroup(b)).map((b) => {
          const on = st.bots !== null && st.bots.includes(b.id);
          return h(
            'button',
            {
              type: 'button',
              class: 'secret-bot' + (on ? ' on' : ''),
              'aria-pressed': on ? 'true' : 'false',
              onclick: () => {
                st.bots = on ? st.bots.filter((x) => x !== b.id) : [...st.bots, b.id];
                draw();
                onChange && onChange();
              },
            },
            avatar(b, 20),
            h('span', null, b.name),
            on ? icon('check', 14) : null
          );
        })
      );
    };
    const sel = selectEl(st.bots === null ? 'all' : 'some', [['all', 'All Bots'], ['some', 'Only the Bots I pick']], (v) => {
      st.bots = v === 'all' ? null : st.bots || [];
      draw();
      onChange && onChange();
    });
    draw();
    wrap.append(sel, list);
    return wrap;
  }
  function tabConnections() {
    const s = S.settings;
    if (!s) return notConnected();
    const ui = S.settingsUI;
    const ready = s.accounts_ready || {};
    const accounts = s.accounts || [];
    const out = h('div');
    const busy = !!ui.connect || !!ui.acctEdit || !!ui.github || !!ui.ghEdit;
    const connectBtn = (p) =>
      h('button', { class: 'btn pill acct-connect', disabled: busy || !ready[p], onclick: () => connectAccount(p) }, providerLogo(p), h('span', null, p === 'google' ? 'Connect Google' : 'Connect Microsoft'));
    out.append(
      h(
        'div',
        { class: 'set-section' },
        h('div', { class: 'set-head' }, h('div', { class: 'set-head-text' }, h('h3', { class: 'set-h3' }, 'Email, calendar and files'), h('div', { class: 'set-explain' }, 'Connect Google or Microsoft so your Bots can read, write and organize email, manage your calendar and, with Google, work with your Google Drive files. You sign in once and stay connected.'))),
        h('div', { class: 'acct-connects' }, connectBtn('google'), connectBtn('microsoft')),
        !ready.google || !ready.microsoft ? h('div', { class: 'f-hint' }, `${[!ready.google && 'Google', !ready.microsoft && 'Microsoft'].filter(Boolean).join(' and ')} sign-in isn’t set up in this version of openbot yet.`) : null
      )
    );
    if (ui.connect) out.append(connectingCard(ui.connect));
    const rows = accounts.map((a) => {
      if (ui.acctEdit && ui.acctEdit.id === a.id) return accountScopeForm(a);
      return row(
        h('span', { class: 'acct-name' }, providerLogo(a.provider), h('span', null, a.email), a.needs_reconnect && h('span', { class: 'chip warn' }, 'Signed out')),
        h(
          'div',
          { class: 'secret-ctl' },
          a.needs_reconnect && pillBtn('Reconnect', () => connectAccount(a.provider, a.id), { disabled: busy }),
          !a.needs_reconnect && a.missing && a.missing.length > 0 && pillBtn('Update permissions', () => connectAccount(a.provider, a.id), { disabled: busy, 'aria-label': `Allow more features for ${a.email}` }),
          pillBtn('Bots', () => ((ui.acctEdit = { id: a.id, bots: a.bots ? a.bots.slice() : null }), rerenderSettings(true)), { disabled: busy, 'aria-label': `Which Bots can use ${a.email}` }),
          pillBtn('Remove', () => removeAccount(a), { disabled: busy, 'aria-label': `Remove ${a.email}` })
        ),
        h(
          'span',
          null,
          h('span', { class: 'secret-desc' }, a.services),
          h('span', { class: 'secret-access' }, scopeText(a.bots, 'Bots')),
          a.needs_reconnect && h('span', { class: 'secret-problem' }, `${PROVIDER_NAME[a.provider]} stopped accepting openbot’s sign-in (for example after a password change). Click Reconnect and sign in again.`),
          !a.needs_reconnect && a.missing && a.missing.length > 0 && h('span', { class: 'secret-access' }, `Not allowed yet: ${a.missing.join(', ')}. Click Update permissions, sign in and tick every box on the permission screen.`)
        )
      );
    });
    out.append(h('div', { class: 'set-section' }, h('div', { class: 'set-label' }, 'Connected accounts'), rows.length ? card(rows) : card(h('div', { class: 'set-empty' }, 'No accounts connected yet'))));
    out.append(note('Sending email, inviting people to events and deleting always ask you first with an Allow / Deny card. openbot keeps the sign-in encrypted on the computer your Bots run on.'));
    out.append(githubSection(s, busy));
    return out;
  }
  // ── Connections: GitHub (github.rs). Sign-in shows a code to enter on github.com. ──
  function githubSection(s, busy) {
    const ui = S.settingsUI;
    const gh = s.github;
    const ready = (s.accounts_ready || {}).github;
    const body = [];
    if (ui.github) body.push(githubCodeCard(ui.github));
    else if (gh && ui.ghEdit) body.push(card(githubScopeForm(gh)));
    else if (gh)
      body.push(
        card(
          row(
            h('span', { class: 'acct-name' }, providerLogo('github'), h('span', null, gh.login), gh.needs_reconnect && h('span', { class: 'chip warn' }, 'Signed out')),
            h(
              'div',
              { class: 'secret-ctl' },
              gh.needs_reconnect && pillBtn('Reconnect', connectGithub, { disabled: busy || !ready }),
              pillBtn('Bots', () => ((ui.ghEdit = { bots: gh.bots ? gh.bots.slice() : null }), rerenderSettings(true)), { disabled: busy, 'aria-label': 'Which Bots can use GitHub' }),
              pillBtn('Remove', removeGithub, { disabled: busy, 'aria-label': 'Remove GitHub' })
            ),
            h(
              'span',
              null,
              h('span', { class: 'secret-desc' }, 'Repos, files, issues and pull requests, including private ones'),
              h('span', { class: 'secret-access' }, scopeText(gh.bots, 'Bots')),
              gh.needs_reconnect && h('span', { class: 'secret-problem' }, 'GitHub stopped accepting openbot’s access (for example after it was removed on github.com). Click Reconnect.')
            )
          )
        )
      );
    else body.push(h('div', { class: 'acct-connects' }, h('button', { class: 'btn pill acct-connect', disabled: busy || !ready, onclick: connectGithub }, providerLogo('github'), h('span', null, 'Connect GitHub'))), !ready ? h('div', { class: 'f-hint' }, 'GitHub sign-in isn’t set up in this version of openbot yet.') : null);
    return h(
      'div',
      { class: 'set-section' },
      h('div', { class: 'set-head' }, h('div', { class: 'set-head-text' }, h('h3', { class: 'set-h3' }, 'GitHub'), h('div', { class: 'set-explain' }, 'Lets your Bots read your repos, issues and pull requests, and work on code. Comments, new issues and pushes always ask you first.'))),
      ...body
    );
  }
  async function connectGithub() {
    const ui = S.settingsUI;
    try {
      const r = await api('/api/github/connect', { method: 'POST' });
      ui.github = { code: r.user_code, url: r.verification_uri || 'https://github.com/login/device', err: '' };
    } catch (e) {
      fail(e);
      return;
    }
    rerenderSettings(true);
  }
  function githubCodeCard(c) {
    const ui = S.settingsUI;
    const copyOpen = async () => {
      try {
        await navigator.clipboard.writeText(c.code);
        toast('Code copied');
      } catch (_) {}
      openExternal(c.url);
    };
    const cancel = () => {
      api('/api/github/connect', { method: 'DELETE' }).catch(() => {});
      ui.github = null;
      rerenderSettings(true);
    };
    return card(
      h(
        'div',
        { class: 'set-row stack acct-wait' },
        c.err
          ? h('div', { class: 'f-err', role: 'alert' }, c.err)
          : h('div', { class: 'acct-wait-head' }, spinner('xs'), h('span', null, 'Enter this code on GitHub')),
        !c.err && h('div', { class: 'gh-code mono', 'aria-label': 'Code: ' + c.code.split('').join(' ') }, c.code),
        !c.err && h('div', { class: 'f-hint' }, `Open ${c.url.replace(/^https:\/\//, '')}, type the code, then click Authorize. This page updates by itself when you’re done.`),
        h('div', { class: 'row-actions' }, btn('Cancel', cancel), c.err ? btn('Try again', () => ((ui.github = null), connectGithub()), 'primary') : btn('Copy code and open GitHub', copyOpen, 'primary'))
      )
    );
  }
  function githubScopeForm(gh) {
    const ui = S.settingsUI;
    const ed = ui.ghEdit;
    const err = h('div', { class: 'f-err', role: 'alert', hidden: true });
    const save = async (e) => {
      if (ed.bots !== null && !ed.bots.length) {
        err.textContent = 'Pick at least one Bot, or choose All Bots';
        err.hidden = false;
        return;
      }
      e.currentTarget.disabled = true;
      try {
        S.settings = await api('/api/github', { method: 'PATCH', body: { bots: ed.bots } });
        ui.ghEdit = null;
      } catch (x) {
        e.currentTarget.disabled = false;
        err.textContent = x.message;
        err.hidden = false;
        return;
      }
      rerenderSettings(true);
    };
    return h(
      'div',
      { class: 'set-row stack secret-form' },
      h('div', { class: 'acct-name' }, providerLogo('github'), h('span', null, gh.login)),
      h('label', { class: 'f-label' }, 'Which Bots can use it'),
      botScopePicker(ed, () => (err.hidden = true)),
      err,
      h('div', { class: 'row-actions' }, btn('Cancel', () => ((ui.ghEdit = null), rerenderSettings(true))), btn('Save', save, 'primary'))
    );
  }
  async function removeGithub() {
    const gh = S.settings && S.settings.github;
    const ok = await confirmDialog({
      title: `Remove GitHub (${gh ? gh.login : ''})?`,
      body: 'Bots can no longer use GitHub, and openbot forgets its access. To also remove openbot from your GitHub account, open github.com › Settings › Applications › Authorized OAuth Apps.',
      confirmText: 'Remove',
      danger: true,
    });
    if (!ok) return;
    try {
      S.settings = await api('/api/github', { method: 'DELETE' });
    } catch (e) {
      fail(e);
    }
    rerenderSettings(true);
  }
  function connectingCard(c) {
    const ui = S.settingsUI;
    const name = PROVIDER_NAME[c.provider];
    const pasteIn = h('input', { class: 'field mono', value: c.paste || '', placeholder: 'http://127.0.0.1:…/oauth/callback?state=…&code=…', spellcheck: 'false', autocomplete: 'off', 'aria-label': 'Address from the browser' });
    const err = h('div', { class: 'f-err', role: 'alert', hidden: !c.err }, c.err || '');
    pasteIn.addEventListener('input', () => ((c.paste = pasteIn.value), (err.hidden = true)));
    const finish = async (e) => {
      const url = pasteIn.value.trim();
      if (!url) return pasteIn.focus();
      e.currentTarget.disabled = true;
      try {
        const r = await api('/api/accounts/finish', { method: 'POST', body: { url } });
        ui.connect = null;
        toast(`${r.email} is connected`);
      } catch (x) {
        c.err = x.message;
        e.currentTarget.disabled = false;
        err.textContent = x.message;
        err.hidden = false;
        return;
      }
      rerenderSettings(true);
    };
    return h(
      'div',
      { class: 'set-section' },
      card(
        h(
          'div',
          { class: 'set-row stack acct-wait' },
          h('div', { class: 'acct-wait-head' }, spinner('xs'), h('span', null, `Sign in to ${name} in your browser`)),
          h('div', { class: 'f-hint' }, c.manual ? 'When you finish, the browser shows an error page. That’s expected: copy the whole address from its address bar and paste it below.' : `Choose your account and allow access. This page updates by itself when you’re done.`),
          h('label', { class: 'f-label' }, c.manual ? 'Address from the browser' : 'Didn’t come back? Paste the address from your browser’s address bar'),
          pasteIn,
          err,
          h(
            'div',
            { class: 'row-actions' },
            btn('Open sign-in again', () => openExternal(c.url)),
            btn('Cancel', () => ((ui.connect = null), rerenderSettings(true))),
            btn('Finish', finish, 'primary')
          )
        )
      )
    );
  }
  function accountScopeForm(a) {
    const ui = S.settingsUI;
    const ed = ui.acctEdit;
    const err = h('div', { class: 'f-err', role: 'alert', hidden: true });
    const save = async (e) => {
      if (ed.bots !== null && !ed.bots.length) {
        err.textContent = 'Pick at least one Bot, or choose All Bots';
        err.hidden = false;
        return;
      }
      e.currentTarget.disabled = true;
      try {
        S.settings = await api('/api/accounts/' + encodeURIComponent(a.id), { method: 'PATCH', body: { bots: ed.bots } });
        ui.acctEdit = null;
      } catch (x) {
        e.currentTarget.disabled = false;
        err.textContent = x.message;
        err.hidden = false;
        return;
      }
      rerenderSettings(true);
    };
    return h(
      'div',
      { class: 'set-row stack secret-form' },
      h('div', { class: 'acct-name' }, providerLogo(a.provider), h('span', null, a.email)),
      h('label', { class: 'f-label' }, 'Which Bots can use it'),
      botScopePicker(ed, () => (err.hidden = true)),
      err,
      h('div', { class: 'row-actions' }, btn('Cancel', () => ((ui.acctEdit = null), rerenderSettings(true))), btn('Save', save, 'primary'))
    );
  }
  async function removeAccount(a) {
    const ok = await confirmDialog({ title: `Remove ${a.email}?`, body: 'Bots can no longer use this account. openbot signs out of it and forgets the sign-in.', confirmText: 'Remove', danger: true });
    if (!ok) return;
    try {
      S.settings = await api('/api/accounts/' + encodeURIComponent(a.id), { method: 'DELETE' });
    } catch (e) {
      fail(e);
    }
    rerenderSettings(true);
  }

  // ── Usage & Billing (1.4 / 6) ──
  function usageInfo() {
    const rl = S.usage && S.usage.rate_limit;
    if (!rl || rl.utilization == null) return null;
    return { pct: Math.round(clamp(rl.utilization, 0, 1.5) * 100), resetsAt: rl.resets_at || null, weekly: rl.window !== 'five_hour' };
  }
  function resetsText(unixSecs) {
    if (!unixSecs) return '';
    const d = new Date(unixSecs * 1000);
    const now = new Date();
    const day0 = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
    const days = Math.round((day0(d) - day0(now)) / 86400000);
    if (days <= 0) return 'Resets today';
    return `Resets in ${days} day${days === 1 ? '' : 's'}`;
  }
  function tabUsage() {
    const s = S.settings;
    const u = S.usage;
    const claude = (S.computer && S.computer.claude) || {};
    const info = usageInfo();
    const out = h('div');
    out.append(
      h(
        'div',
        { class: 'set-section', id: 'set-usage' },
        h(
          'div',
          { class: 'set-card usage-card' },
          h('div', { class: 'usage-title' }, info && !info.weekly ? '5-hour usage limit' : 'Weekly usage limit'),
          info ? h('div', { class: 'usage-big' }, h('span', { class: 'num' }, `${info.pct}%`), h('span', { class: 'unit' }, 'used')) : h('div', { class: 'usage-big' }, h('span', { class: 'num dim' }, '—')),
          h('div', { class: 'meter big' }, h('div', { class: 'meter-fill', style: { width: clamp(info ? info.pct : 0, 0, 100) + '%' } })),
          h('div', { class: 'usage-sub' }, info ? resetsText(info.resetsAt) : 'Shows up after your Bots’ next reply.')
        ),
        note(info && !info.weekly ? 'Included usage with your subscription. Resets every 5 hours.' : 'Included usage with your subscription. Resets weekly.')
      )
    );
    const plan = claude.subscription ? claude.subscription[0].toUpperCase() + claude.subscription.slice(1) : '';
    out.append(
      section(
        'Manage Plan',
        card(row(plan ? `Current plan: ${plan}` : 'Current plan', pillBtn(h('span', { class: 'btn-ext' }, 'Manage Plan', icon('external', 13)), () => openExternal(USAGE_URL)), 'View, change, or upgrade your plan on claude.ai'))
      )
    );
    if (s) {
      const keyIn = h('input', { class: 'field', type: 'password', placeholder: s.api_key_set ? 'Enter a new key to replace it' : 'sk-ant-…', autocomplete: 'off' });
      const saveKey = () => {
        const v = keyIn.value.trim();
        if (!v) return keyIn.focus();
        putSettings({ api_key: v, auth_mode: 'api_key' }).then((ok) => ok && toast('API key saved'));
      };
      keyIn.addEventListener('keydown', (e) => e.key === 'Enter' && saveKey());
      const radio = (on, title, sub, onchange) =>
        h(
          'label',
          { class: 'set-row radio-row' + (on ? ' on' : '') },
          h('input', { type: 'radio', name: 'auth', checked: on, onchange }),
          h('span', { class: 'radio-dot' }),
          h('div', { class: 'set-row-text' }, h('div', { class: 'set-row-title' }, title), h('div', { class: 'set-row-sub' }, sub))
        );
      out.append(
        section(
          'Sign-in method',
          card(
            radio(
              s.auth_mode !== 'api_key',
              'Claude subscription',
              claude.logged_in ? `Signed in to Claude Code as ${claude.email || 'your account'}${claude.subscription ? ` · ${claude.subscription}` : ''}` : claude.installed ? 'Claude Code isn’t signed in on that computer. Open a terminal there, run claude, and log in.' : 'Install Claude Code on the computer where bots run, then run claude and log in.',
              () => putSettings({ auth_mode: 'subscription' })
            ),
            radio(s.auth_mode === 'api_key', 'Anthropic API key', s.api_key_locked ? 'The saved key can’t be unlocked on this computer. Enter it again.' : s.api_key_set ? `Key saved: ${s.api_key_hint || '••••'}` : 'Pay per use with a key from console.anthropic.com.', () => {
              if (s.api_key_set) putSettings({ auth_mode: 'api_key' });
              else {
                keyIn.focus();
                toast('Paste your API key and press Save');
              }
            }),
            h('div', { class: 'set-row' }, h('div', { class: 'inline-form grow' }, keyIn, btn('Save', saveKey, 'primary'), s.api_key_set && btn('Remove', () => putSettings({ api_key: '', auth_mode: 'subscription' }).then((ok) => ok && toast('API key removed')), 'danger')))
          ),
          note('The key is stored only on the computer where bots run.')
        )
      );
    }
    if (u && (u.per_bot || []).length) {
      const per = u.per_bot.slice().sort((a, b) => (b.cost_usd || 0) - (a.cost_usd || 0));
      out.append(
        section(
          'Usage by Bot',
          card(
            per.map((p) => {
              const b = S.bots.get(p.bot_id);
              return row(h('span', { class: 'bot-inline' }, avatar(b || {}, 22), h('span', null, b ? b.name : 'Deleted Bot')), h('span', { class: 'val' }, fmtUsd(p.cost_usd)), `${p.turns || 0} turn${p.turns === 1 ? '' : 's'}`);
            })
          ),
          note(`This week ${fmtUsd(u.week_cost_usd)} · all time ${fmtUsd(u.total_cost_usd)}. Estimated by Claude Code. On a subscription you aren’t charged this. It shows what the same usage would cost with an API key.`)
        )
      );
    }
    return out;
  }
  // chat banner at 75 / 90 / 100 % of the weekly limit (1.4)
  function renderUsageBanner() {
    const el = $('#usageBanner');
    if (!el) return;
    const info = usageInfo();
    let th = 0;
    if (info && info.weekly) th = info.pct >= 100 ? 100 : info.pct >= 90 ? 90 : info.pct >= 75 ? 75 : 0;
    const key = th ? `${th}:${(info && info.resetsAt) || ''}` : '';
    let dismissed = '';
    try {
      dismissed = localStorage.getItem('openbot.usageWarning') || '';
    } catch {}
    const show = !!th && dismissed !== key;
    const was = !el.hidden;
    el.hidden = !show;
    if (show) {
      el.textContent = '';
      el.append(
        icon('alert', 15),
        h('span', { class: 'ub-text' }, th === 100 ? 'Weekly usage limit reached.' : `You’re at ${info.pct}% of your weekly usage limit.`),
        h('button', { class: 'btn small primary', onclick: () => openExternal(USAGE_URL) }, th === 100 ? 'Upgrade' : 'Get More Usage'),
        h(
          'button',
          {
            class: 'ghost-btn sm',
            'aria-label': 'Dismiss usage warning',
            onclick: () => {
              try {
                localStorage.setItem('openbot.usageWarning', key);
              } catch {}
              renderUsageBanner();
            },
          },
          icon('x', 14)
        )
      );
    }
    if (was !== show) layoutComposer();
  }

  // ── Updates (1.5) ──
  function tabUpdates() {
    const claude = (S.computer && S.computer.claude) || {};
    const ui = S.settingsUI;
    const up = ui.upd;
    const cv = (claude.version || '').replace(/^v/, '').split(' ')[0];
    const check = async () => {
      up.busy = true;
      up.status = 'Checking for updates…';
      rerenderSettings(true);
      try {
        const r = await api('/api/claude/update', { method: 'POST' });
        const text = (r && r.output) || '';
        const last = text.split('\n').map((l) => l.trim()).filter(Boolean).pop() || '';
        up.status = r && r.ok ? (/up to date|latest/i.test(text) ? 'You’re up to date' : last || 'Claude Code was updated') : last || 'Couldn’t check for updates. Check your connection and try again.';
        up.bad = !(r && r.ok);
        api('/api/computer').then((c) => {
          S.computer = c;
          rerenderSettings();
        });
      } catch (e) {
        up.status = e && e.network ? 'Couldn’t check for updates. Check your connection and try again.' : e.message;
        up.bad = true;
      }
      up.busy = false;
      rerenderSettings(true);
    };
    const copyBtn = (v) => h('button', { class: 'ghost-btn sm', 'aria-label': 'Copy version', 'data-tip': 'Copy version', onclick: () => copyText(v) }, icon('copy', 14));
    return h(
      'div',
      null,
      section('openbot Updates', card(row('Version', h('span', { class: 'ver' }, `Version ${S.version || '—'}`, S.version && copyBtn(S.version)), 'openbot updates when you install a new build.', '', 'update-status'))),
      section(
        'Claude Code Updates',
        card(
          row(
            'Version',
            h('span', { class: 'ver' }, claude.installed ? `Version ${cv || '?'}` : 'Not installed', claude.installed && cv && copyBtn(cv), claude.installed && pillBtn(up.busy ? 'Checking…' : 'Check for Updates', check, { disabled: !!up.busy })),
            up.status ? h('span', { class: up.bad ? 'err' : '' }, up.status) : claude.installed ? 'Claude Code also updates itself in the background.' : 'Install Claude Code on the computer where Bots run.'
          )
        )
      )
    );
  }

  // ───────────────────────────── onboarding · always-on computer setup · connection states ─────────────────────────────
  const UA = navigator.platform || navigator.userAgent || '';
  const PLATFORM = /Mac|iPhone|iPad/.test(UA) ? 'mac' : /Win/.test(UA) ? 'win' : 'other';
  const cap = (s) => (s ? s[0].toUpperCase() + s.slice(1) : s);
  const thisDevice = () => (PLATFORM === 'mac' ? 'this Mac' : PLATFORM === 'win' ? 'this PC' : 'this computer');
  const ThisDevice = () => cap(thisDevice());
  const CLAUDE_INSTALL = { unix: 'curl -fsSL https://claude.ai/install.sh | bash', win: 'irm https://claude.ai/install.ps1 | iex' };
  const HOST_CMD = 'Set-ExecutionPolicy -Scope Process Bypass; .\\install-host.ps1 -KeepAwake';
  const GITHUB_URL = 'https://github.com/bayshores/openbot';
  const TAILSCALE_URL = 'https://tailscale.com/download';
  function osLabel(os) {
    const o = String(os || '').toLowerCase();
    if (o.startsWith('win')) return 'Windows';
    if (o.includes('mac') || o === 'darwin') return 'Mac';
    if (o === 'ios') return 'iPhone';
    if (o.includes('android')) return 'Android';
    if (o.includes('linux')) return 'Linux';
    return os || 'Device';
  }
  function prettyHost(u) {
    const host = hostOf(u);
    return /\.ts\.net(:\d+)?$/i.test(host) ? host.split('.')[0].toUpperCase() : host;
  }
  // The name of the computer the Bots run on, as people say it.
  function computerName() {
    if (S.conn.mode !== 'remote') return thisDevice();
    return ((S.prefs.remoteNames || {})[S.conn.url]) || prettyHost(S.conn.url);
  }
  // Does the computer the Bots use run Windows? (picks PowerShell vs Terminal wording)
  function botComputerIsWindows() {
    if (S.conn.mode === 'remote') return /^win/i.test((S.computer && S.computer.os) || '') || /^win-/i.test(computerName());
    return PLATFORM === 'win';
  }
  const shellName = () => (botComputerIsWindows() ? 'PowerShell' : 'Terminal');

  // small building blocks shared by the onboarding and setup dialogs
  function cmdBlock(cmd) {
    return h(
      'div',
      { class: 'ob-cmd' },
      h('code', null, cmd),
      h('button', { class: 'ghost-btn sm', 'aria-label': 'Copy command', 'data-tip': 'Copy', onclick: () => copyText(cmd, 'Copied') }, icon('copy', 14))
    );
  }
  function statusLine(kind, ...text) {
    return h('div', { class: 'ob-status ' + kind }, kind === 'wait' ? spinner('xs') : icon(kind === 'ok' ? 'check' : 'alert', 15, '', 2.2), h('span', null, ...text));
  }
  const extLink = (label, url) => h('button', { class: 'link-btn ob-link', onclick: () => openExternal(url) }, label, icon('external', 12));

  // ── "Is Claude Code ready?" (onboarding step, banner help, send guard) ──
  const claudeReady = () => {
    const c = S.computer && S.computer.claude;
    return !c || (c.installed && c.logged_in);
  };
  async function recheckClaude() {
    try {
      S.computer = await api('/api/computer?refresh=1');
    } catch (e) {
      if (e && e.network) noteNetworkTrouble();
    }
    renderSysBanner();
    if (S.settingsUI) rerenderSettings();
  }
  function claudeCheckView(onChange) {
    const box = h('div', { class: 'ob-check' });
    let busy = false;
    const paint = () => {
      box.textContent = '';
      const c = S.computer && S.computer.claude;
      const where = computerName();
      const again = h('button', { class: 'btn', disabled: busy, onclick: check }, busy ? 'Checking…' : 'Check again');
      if (!c) {
        box.append(statusLine('wait', 'Checking Claude Code…'));
        return;
      }
      if (!c.installed) {
        box.append(
          statusLine('bad', `Claude Code isn’t installed on ${where}`),
          h('p', { class: 'ob-p' }, `openbot uses Claude Code to do the work. Open ${shellName()} on ${where}, paste this, and press Enter:`),
          cmdBlock(botComputerIsWindows() ? CLAUDE_INSTALL.win : CLAUDE_INSTALL.unix),
          h('p', { class: 'ob-p' }, 'When it finishes, type ', h('code', null, 'claude'), ' and sign in with your Claude account.'),
          h('div', { class: 'ob-row' }, again)
        );
      } else if (!c.logged_in) {
        box.append(
          statusLine('bad', `Claude isn’t signed in on ${where}`),
          h(
            'ol',
            { class: 'ob-steps' },
            h('li', null, `Open ${shellName()} on ${where}.`),
            h('li', null, 'Type ', h('code', null, 'claude'), ' and press Enter.'),
            h('li', null, 'Sign in with your Claude account, then come back here.')
          ),
          h('div', { class: 'ob-row' }, again, h('button', { class: 'link-btn', onclick: () => openSettings('usage') }, 'Or use an API key instead'))
        );
      } else {
        box.append(statusLine('ok', `Connected as ${c.email || 'your Claude account'}${c.subscription ? ` (${cap(c.subscription)})` : ''}`));
      }
    };
    async function check() {
      busy = true;
      paint();
      await recheckClaude();
      busy = false;
      paint();
      if (onChange) onChange();
    }
    paint();
    return { el: box, paint, check };
  }
  function openClaudeHelp() {
    const view = claudeCheckView(() => claudeReady() && setTimeout(() => close(), 900));
    const box = h(
      'div',
      { class: 'dialog', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Set up Claude Code' },
      h('button', { class: 'icon-btn dialog-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 16)),
      h('h2', { class: 'dialog-title' }, 'Set up Claude Code'),
      h('p', { class: 'dialog-body' }, `Bots use Claude Code on ${computerName()} with your Claude subscription. It takes about a minute.`),
      view.el,
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn primary', onclick: () => close() }, 'Done'))
    );
    const close = openModal(box);
  }

  // ── "Set up an always-on computer": 3-step tutorial ──
  // opts: {step: 0|1|2, url, onDone(name), onLater(), embedded}
  function setupWizard(opts = {}) {
    const st = { step: opts.step || 0, ts: null, tsErr: false, tsBusy: false, disc: null, discBusy: false, sel: null, manual: !!opts.url, url: opts.url || 'https://', code: '', busy: false, err: '' };
    const root = h('div', { class: 'wz' });
    const STEP_NAMES = [`Tailscale on ${thisDevice()}`, 'The other computer', 'Connect'];
    async function loadTs() {
      st.tsBusy = true;
      paint();
      try {
        st.ts = await home('/api/tailscale');
        st.tsErr = false;
      } catch {
        st.tsErr = true;
      }
      st.tsBusy = false;
      paint();
    }
    async function discover() {
      st.discBusy = true;
      st.err = '';
      paint();
      try {
        const r = await home('/api/discover', { method: 'POST' });
        st.disc = (r && r.found) || [];
        if (r && r.tailscale) st.ts = r.tailscale;
      } catch {
        st.disc = [];
      }
      if (st.disc.length && !st.manual) st.sel = st.sel && st.disc.some((f) => f.url === st.sel) ? st.sel : st.disc[0].url;
      if (!st.disc.length) st.manual = true;
      st.discBusy = false;
      paint();
    }
    async function connect() {
      const code = st.code.replace(/\D/g, '');
      let url = st.manual || !st.sel ? st.url.trim() : st.sel;
      if (url && !/^https?:\/\//i.test(url)) url = 'https://' + url;
      url = url.replace(/\/+$/, '');
      st.err = '';
      if (!/^https?:\/\/[^/]+/.test(url) || /^https?:\/\/$/.test(url)) {
        st.err = 'Enter the address the other computer showed, like https://my-pc.tailnet.ts.net';
        return paint();
      }
      if (code.length !== 6) {
        st.err = 'Enter the 6-digit code the other computer showed.';
        return paint();
      }
      st.busy = true;
      paint();
      try {
        const r = await home('/api/connection/pair', { method: 'POST', body: { url, code } });
        if (!r || !r.ok || !r.token) throw new Error((r && r.error) || 'That code didn’t work. Check it and try again.');
        const found = (st.disc || []).find((f) => f.url === url);
        const name = (r.computer && r.computer.hostname) || (found && (found.computer_name || found.name)) || prettyHost(url);
        S.conn = await home('/api/connection', { method: 'PUT', body: { mode: 'remote', url, token: r.token } });
        S.prefs.remoteNames = Object.assign({}, S.prefs.remoteNames, { [S.conn.url || url]: name });
        await savePrefsNow();
        S.connDraft = null;
        S.current = null;
        await boot();
        toast(`Connected to ${name}`);
        st.busy = false;
        if (opts.onDone) opts.onDone(name);
      } catch (e) {
        st.busy = false;
        st.err = e && e.network ? 'Couldn’t reach openbot on this computer. Is the openbot app still running?' : e.status === 404 ? 'Couldn’t find openbot at that address. Check it and try again.' : String(e.message || '').replace(/`/g, '');
        paint();
      }
    }
    function header() {
      return h(
        'ol',
        { class: 'wz-steps', 'aria-label': 'Steps' },
        STEP_NAMES.map((n, i) =>
          h(
            'li',
            { class: 'wz-step' + (i === st.step ? ' on' : i < st.step ? ' done' : ''), 'aria-current': i === st.step ? 'step' : null },
            h('span', { class: 'wz-num' }, i < st.step ? icon('check', 12, '', 2.6) : String(i + 1)),
            h('span', { class: 'wz-name' }, n)
          )
        )
      );
    }
    function stepTailscale() {
      const t = st.ts;
      const again = h('button', { class: 'btn', disabled: st.tsBusy, onclick: loadTs }, st.tsBusy ? 'Checking…' : 'Check again');
      let status;
      if (!t && (st.tsBusy || !st.tsErr)) status = [statusLine('wait', 'Checking Tailscale…')];
      else if (st.tsErr) status = [statusLine('bad', 'Couldn’t check Tailscale on ', thisDevice(), '.'), h('div', { class: 'ob-row' }, again)];
      else if (!t.installed) status = [statusLine('bad', `Tailscale isn’t installed on ${thisDevice()}`), h('div', { class: 'ob-row' }, h('button', { class: 'btn primary', onclick: () => openExternal(TAILSCALE_URL) }, 'Download Tailscale', icon('external', 13)), again)];
      else if (t.needs_login || !t.running) status = [statusLine('bad', 'Tailscale is installed but not signed in'), h('p', { class: 'ob-p' }, 'Open Tailscale (in the menu bar on a Mac) and sign in. Use the same account you’ll use on the other computer.'), h('div', { class: 'ob-row' }, again)];
      else status = [statusLine('ok', `Tailscale is on (this device: ${(t.self && t.self.name) || thisDevice()})`)];
      return [
        h('h3', { class: 'wz-title' }, `Turn on Tailscale on ${thisDevice()}`),
        h('p', { class: 'ob-p' }, `Tailscale makes a private, encrypted link between your own devices, so ${thisDevice()} can reach your always-on computer from anywhere. It’s free for personal use.`),
        ...status,
      ];
    }
    function stepOther() {
      const t = st.ts;
      const peers = ((t && t.peers) || []).slice().sort((a, b) => (b.online ? 1 : 0) - (a.online ? 1 : 0));
      const list = peers.length
        ? h(
            'div',
            { class: 'wz-card' },
            peers.map((p) =>
              h(
                'div',
                { class: 'wz-dev' },
                h('span', { class: 'wz-os' }, icon(/ios|android/i.test(p.os || '') ? 'phone' : 'monitor', 16)),
                h('span', { class: 'wz-dev-text' }, h('span', { class: 'wz-dev-name' }, p.name || p.dns), h('span', { class: 'wz-dev-sub' }, osLabel(p.os))),
                h('span', { class: 'status-pill small ' + (p.online ? 'on' : 'off') }, h('i'), p.online ? 'Online' : 'Offline')
              )
            )
          )
        : h('div', { class: 'wz-card' }, h('div', { class: 'p-empty' }, t ? 'No other devices yet. They show up here once Tailscale is signed in on them.' : 'Checking your devices…'));
      return [
        h('h3', { class: 'wz-title' }, 'Set up the other computer'),
        h('p', { class: 'ob-p' }, 'Use a computer that stays on, like a Windows PC at home. Do these steps on that computer:'),
        h(
          'ol',
          { class: 'ob-steps big' },
          h('li', null, 'Install Tailscale and sign in with the same account. ', extLink('Get Tailscale', TAILSCALE_URL)),
          h('li', null, 'Install Claude Code: open PowerShell, paste this, press Enter, then type ', h('code', null, 'claude'), ' and sign in.', cmdBlock(CLAUDE_INSTALL.win)),
          h('li', null, 'Download ', h('code', null, 'openbot-server.exe'), ' and ', h('code', null, 'install-host.ps1'), ' from the openbot GitHub page into one folder. ', extLink('Open GitHub', GITHUB_URL)),
          h('li', null, 'Open PowerShell as Administrator in that folder and run:', cmdBlock(HOST_CMD)),
          h('li', null, 'It shows an address and a 6-digit code. Keep them for the next step.')
        ),
        h('div', { class: 'wz-sub' }, h('span', null, 'Your devices on Tailscale'), h('button', { class: 'link-btn', onclick: loadTs }, st.tsBusy ? 'Checking…' : 'Refresh')),
        list,
      ];
    }
    function stepConnect() {
      const out = [h('h3', { class: 'wz-title' }, 'Connect to it'), h('p', { class: 'ob-p' }, 'Pick your always-on computer and enter the 6-digit code it showed.')];
      if (st.discBusy) out.push(statusLine('wait', 'Looking for your computers…'));
      else if (st.disc && st.disc.length && !st.manual) {
        out.push(
          h(
            'div',
            { class: 'radio-list wz-found', role: 'radiogroup', 'aria-label': 'Computers running openbot' },
            st.disc.map((f) =>
              h(
                'button',
                {
                  class: 'radio-item' + (st.sel === f.url ? ' on' : ''),
                  role: 'radio',
                  'aria-checked': st.sel === f.url ? 'true' : 'false',
                  onclick: () => {
                    st.sel = f.url;
                    paint();
                  },
                },
                h('span', { class: 'wz-os' }, icon('monitor', 16)),
                h('span', { class: 'radio-item-name' }, [f.computer_name || f.name, osLabel(f.os), f.version ? `openbot ${f.version}` : ''].filter(Boolean).join(' · ')),
                h('span', { class: 'radio-dot' })
              )
            )
          ),
          h('div', { class: 'ob-row' }, h('button', { class: 'link-btn', onclick: () => ((st.manual = true), paint()) }, 'Enter the address yourself'))
        );
      } else {
        if (st.disc && !st.disc.length) out.push(statusLine('bad', 'Didn’t find openbot on your devices yet'), h('div', { class: 'ob-row' }, h('button', { class: 'btn', onclick: discover }, 'Search again')));
        const urlIn = h('input', { class: 'field mono', value: st.url, placeholder: 'https://my-pc.tailnet.ts.net', spellcheck: 'false', autocomplete: 'off', 'aria-label': 'Computer address' });
        urlIn.addEventListener('input', () => (st.url = urlIn.value));
        out.push(h('label', { class: 'f-label' }, 'Address'), urlIn);
        if (st.disc && st.disc.length) out.push(h('div', { class: 'ob-row' }, h('button', { class: 'link-btn', onclick: () => ((st.manual = false), paint()) }, `Pick from the ${st.disc.length} found`)));
      }
      const codeIn = h('input', { class: 'field wz-code', value: st.code, placeholder: '123 456', inputmode: 'numeric', autocomplete: 'one-time-code', maxlength: '7', spellcheck: 'false', 'aria-label': '6-digit code' });
      codeIn.addEventListener('input', () => {
        const d = codeIn.value.replace(/\D/g, '').slice(0, 6);
        codeIn.value = d.length > 3 ? d.slice(0, 3) + ' ' + d.slice(3) : d;
        st.code = codeIn.value;
        if (st.err) {
          st.err = '';
          errEl.hidden = true;
        }
      });
      codeIn.addEventListener('keydown', (e) => e.key === 'Enter' && connect());
      const errEl = h('div', { class: 'dialog-error', role: 'alert', hidden: !st.err }, st.err);
      out.push(h('label', { class: 'f-label' }, '6-digit code'), codeIn, errEl);
      st.codeIn = codeIn;
      return out;
    }
    function paint() {
      const ae = document.activeElement;
      const refocus = ae && root.contains(ae) && ae.getAttribute('aria-label');
      root.textContent = '';
      const body = h('div', { class: 'wz-body' }, ...[stepTailscale, stepOther, stepConnect][st.step]());
      const back = st.step > 0 || opts.onBack ? h('button', { class: 'btn', onclick: () => (st.step > 0 ? goStep(st.step - 1) : opts.onBack()) }, 'Back') : null;
      const tsOk = st.ts && st.ts.installed && st.ts.running && !st.ts.needs_login;
      const next =
        st.step < 2
          ? h('button', { class: 'btn primary', onclick: () => goStep(st.step + 1) }, st.step === 0 && !tsOk ? 'Continue anyway' : 'Next')
          : h('button', { class: 'btn primary', disabled: st.busy || st.discBusy, onclick: connect }, st.busy ? 'Connecting…' : 'Connect');
      root.append(
        header(),
        body,
        h('div', { class: 'dialog-actions wz-foot' }, opts.onLater && h('button', { class: 'link-btn', onclick: opts.onLater }, 'Set up later'), h('span', { style: 'flex:1' }), back, next)
      );
      if (refocus) {
        const el = root.querySelector(`[aria-label="${refocus}"]`);
        if (el) el.focus();
      }
    }
    function goStep(n) {
      st.step = n;
      st.err = '';
      paint();
      if (n === 2) {
        if (!st.disc && !st.discBusy && !opts.url) discover();
        setTimeout(() => st.codeIn && !st.discBusy && (st.manual ? root.querySelector('[aria-label="Computer address"]') || st.codeIn : st.codeIn).focus(), 30);
      }
      if (opts.onStep) opts.onStep(n);
    }
    paint();
    loadTs();
    if (st.step === 2) goStep(2);
    return { el: root, goStep };
  }
  function openSetupWizard(opts = {}) {
    const box = h('div', { class: 'dialog wide wz-dialog', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Set up an always-on computer' });
    const close = openModal(box);
    const wz = setupWizard(Object.assign({}, opts, { onDone: () => close() }));
    box.append(
      h('button', { class: 'icon-btn dialog-close', onclick: () => close(), 'aria-label': 'Close' }, icon('x', 16)),
      h('h2', { class: 'dialog-title' }, opts.url ? `Pair ${thisDevice()} again` : 'Set up an always-on computer'),
      h('p', { class: 'dialog-body' }, opts.url ? 'Get a new 6-digit code on that computer: open openbot there and choose Settings › Computer › Pair a new device, or run openbot-server pair.' : 'Your Bots keep working 24/7 on a computer that stays on, and you use them from here.'),
      wz.el
    );
    return close;
  }

  // ── first-run onboarding (settings-and-bots.md 4.4; avatar moods per avatars.md "Onboarding steps") ──
  const SUGGESTIONS = ['What can you do for me?', 'Every weekday at 8am, send me a short news summary', 'Help me plan my week'];
  function maybeOnboard() {
    if (S.prefs.onboarded || S.onboardingOpen) return;
    // People who already use openbot skip it.
    const used = !S.freshPrefs || S.bots.size > 1 || S.conn.mode === 'remote';
    if (used) {
      S.prefs.onboarded = true;
      savePrefs();
      return;
    }
    openOnboarding();
  }
  function openOnboarding() {
    if (S.onboardingOpen) return;
    const mainBot = () => realBots().find((b) => b.main) || orderedBots().find((b) => !isGroup(b)) || null;
    const mb = mainBot();
    const look = mb ? lookOf(mb) : { shape: 'ball', color: 'violet' };
    const st = { i: 0, where: S.conn.mode === 'remote' ? 'remote' : 'local', name: S.prefs.userName || '', wz: null };
    const av = h('span', { class: 'av', style: { width: '96px', height: '96px' } });
    av._size = 96;
    paintAvatar(av, look, 96, { live: true, interactive: true });
    av.addEventListener('click', () => pokeAvatar(av));
    const art = h('div', { class: 'ob-art' }, av);
    const body = h('div', { class: 'ob-body' });
    const foot = h('div', { class: 'ob-foot' });
    const dots = h('div', { class: 'ob-dots', 'aria-hidden': 'true' });
    const skip = h('button', { class: 'link-btn ob-skip', onclick: () => finish() }, 'Skip');
    const box = h('div', { class: 'dialog onboard', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Welcome to openbot' }, skip, art, body, foot, dots);
    const onMove = (e) => {
      if (!av._av || !av._av.lookAt) return;
      const r = av.getBoundingClientRect();
      const nx = clamp((e.clientX - (r.left + r.width / 2)) / (innerWidth / 2), -1, 1);
      const ny = clamp((e.clientY - (r.top + r.height / 2)) / (innerHeight / 2), -1, 1);
      try {
        av._av.lookAt(nx, ny);
      } catch {}
    };
    window.addEventListener('pointermove', onMove);
    S.onboardingOpen = true;
    const close = openModal(box, {
      cls: 'onboard-overlay',
      dismissable: false,
      onClose: () => {
        window.removeEventListener('pointermove', onMove);
        S.onboardingOpen = false;
      },
    });
    const steps = () => ['welcome', 'name', 'where', ...(st.where === 'remote' && S.conn.mode !== 'remote' ? ['setup'] : []), 'claude', 'done'];
    const mood = (s) => setAvatarState(av, s, true);
    function go(i) {
      st.i = clamp(i, 0, steps().length - 1);
      paint();
    }
    function finish(suggestion) {
      if (st.name.trim() !== (S.prefs.userName || '')) S.prefs.userName = st.name.trim();
      S.prefs.onboarded = true;
      savePrefs();
      renderMe();
      close();
      const b = mainBot();
      if (b) {
        selectBot(b.id, { focus: true });
        if (suggestion) {
          input.value = suggestion;
          updateComposer();
          input.focus();
        }
      }
    }
    const nav = (nextLabel = 'Next', nextFn = () => go(st.i + 1), extra = null) =>
      h('div', { class: 'ob-nav' }, st.i > 0 ? h('button', { class: 'btn', onclick: () => go(st.i - 1) }, 'Back') : h('span'), extra, h('button', { class: 'btn primary', onclick: nextFn }, nextLabel));
    function paint() {
      const id = steps()[st.i];
      body.textContent = '';
      foot.textContent = '';
      box.classList.toggle('wide', id === 'setup');
      art.hidden = id === 'setup';
      skip.hidden = id === 'done';
      dots.textContent = '';
      steps().forEach((_, j) => dots.append(h('span', { class: j === st.i ? 'on' : '' })));
      if (id === 'welcome') {
        mood('idle');
        body.append(h('h1', { class: 'ob-title' }, 'openbot'), h('p', { class: 'ob-lead' }, 'Your always-on Bots that do real work on a computer, powered by your Claude subscription.'));
        foot.append(h('button', { class: 'btn primary ob-wide', onclick: () => go(1) }, 'Get started'));
      } else if (id === 'name') {
        mood('waking');
        const nameIn = h('input', { class: 'field ob-name', value: st.name, placeholder: 'Your name', maxlength: '40', 'aria-label': 'Your name', autocomplete: 'given-name' });
        nameIn.addEventListener('input', () => (st.name = nameIn.value));
        nameIn.addEventListener('keydown', (e) => e.key === 'Enter' && go(st.i + 1));
        body.append(h('h1', { class: 'ob-title' }, mb ? `Hi, I’m ${mb.name}` : 'Hi there'), h('p', { class: 'ob-lead' }, 'What should your Bots call you?'), nameIn, h('div', { class: 'f-hint center' }, 'Optional. You can change it later in Settings.'));
        foot.append(nav());
        setTimeout(() => nameIn.focus(), 60);
      } else if (id === 'where') {
        mood('thinking');
        setTimeout(() => steps()[st.i] === 'where' && mood('idle'), 1400);
        const choice = (key, title, sub, ic) =>
          h(
            'button',
            {
              class: 'ob-choice' + (st.where === key ? ' on' : ''),
              role: 'radio',
              'aria-checked': st.where === key ? 'true' : 'false',
              onclick: () => {
                st.where = key;
                paint();
              },
            },
            h('span', { class: 'ob-choice-ic' }, icon(ic, 20)),
            h('span', { class: 'ob-choice-text' }, h('span', { class: 'ob-choice-title' }, title), h('span', { class: 'ob-choice-sub' }, sub)),
            h('span', { class: 'radio-dot' })
          );
        const remoteNow = S.conn.mode === 'remote';
        body.append(
          h('h1', { class: 'ob-title' }, 'Where should your Bots work?'),
          h('p', { class: 'ob-lead' }, 'Bots use a real computer to open files, browse, and run tasks.'),
          h(
            'div',
            { class: 'ob-choices', role: 'radiogroup', 'aria-label': 'Where should your Bots work?' },
            choice('local', ThisDevice(), `Works right now. Bots pause when ${thisDevice()} sleeps or is closed.`, 'monitor'),
            choice('remote', 'An always-on computer', remoteNow ? `Connected to ${computerName()}. Bots keep working 24/7.` : 'Like a Windows PC at home. Bots keep working 24/7, even when this one is off.', 'bolt')
          )
        );
        const toLocal = async () => {
          if (st.where === 'local' && S.conn.mode === 'remote') await useThisDevice(true);
          go(st.i + 1);
        };
        foot.append(nav(st.where === 'remote' && !remoteNow ? 'Set it up' : 'Next', toLocal));
      } else if (id === 'setup') {
        mood('working');
        if (!st.wz)
          st.wz = setupWizard({
            onBack: () => go(st.i - 1),
            onLater: () => {
              st.where = 'local';
              st.wz = null;
              go(steps().indexOf('claude'));
            },
            onDone: () => {
              st.wz = null;
              go(steps().indexOf('claude'));
            },
          });
        body.append(h('h1', { class: 'ob-title small' }, 'Set up an always-on computer'), st.wz.el);
      } else if (id === 'claude') {
        mood('working');
        const view = claudeCheckView(() => {
          mood(claudeReady() ? 'happy' : 'working');
          paintFoot();
        });
        body.append(h('h1', { class: 'ob-title' }, 'Connect your Claude account'), h('p', { class: 'ob-lead' }, `Bots do their work with Claude Code on ${computerName()}, using your Claude subscription.`), view.el);
        const paintFoot = () => {
          foot.textContent = '';
          foot.append(nav(claudeReady() ? 'Next' : 'Continue anyway'));
        };
        paintFoot();
        view.check();
      } else if (id === 'done') {
        mood('excited');
        setTimeout(() => S.onboardingOpen && mood('celebrate'), 1800);
        const b = mainBot();
        appendKids(body, [
          h('h1', { class: 'ob-title' }, 'You’re all set'),
          h('p', { class: 'ob-lead' }, b ? `${b.name} is your main Bot. Say hi, or try one of these:` : 'Create your first Bot with the + button.'),
          b && h('div', { class: 'ob-suggest' }, SUGGESTIONS.map((t) => h('button', { class: 'ob-chip', onclick: () => finish(t) }, t))),
        ]);
        foot.append(h('button', { class: 'btn primary ob-wide', onclick: () => finish() }, 'Start chatting'));
      }
    }
    paint();
  }

  // ── connection + Claude status around the app (settings-and-bots.md 1.7.5 sidebar banners) ──
  async function useThisDevice(quiet) {
    try {
      S.conn = await home('/api/connection', { method: 'PUT', body: { mode: 'local', url: '', token: '' } });
      S.connDraft = null;
      S.current = null;
      S.link = 'ok';
      await boot();
      if (!quiet) toast(`Bots now run on ${thisDevice()}`);
    } catch (e) {
      fail(e);
    }
  }
  async function probeLink() {
    const ctl = new AbortController();
    const t = setTimeout(() => ctl.abort(), 6000);
    try {
      const r = await fetch(S.apiBase + '/api/computer', { headers: S.apiToken ? { Authorization: 'Bearer ' + S.apiToken } : {}, signal: ctl.signal });
      if (r.status === 401) return 'unpaired';
      if (!r.ok) return 'unreachable';
      try {
        S.computer = await r.json();
      } catch {}
      return 'ok';
    } catch {
      return 'unreachable';
    } finally {
      clearTimeout(t);
    }
  }
  function setLink(v) {
    if (S.link === v) return;
    S.link = v;
    renderSysBanner();
    if (S.settingsUI) rerenderSettings();
  }
  // a request failed because the computer can't be reached: show the banner instead of piling up toasts
  let probing = false;
  async function noteNetworkTrouble(status) {
    if (status === 401) return setLink('unpaired');
    if (probing || !S.booted) return;
    probing = true;
    const v = await probeLink();
    probing = false;
    if (v !== 'ok') setLink(v);
  }
  function retryNow() {
    S.link = 'retrying';
    renderSysBanner();
    S.wsRetry = 1000;
    probeLink().then((v) => {
      if (v === 'ok') {
        if (!S.wsOk) {
          closeWS();
          connectWS();
        }
        setLink('ok');
        resync();
      } else setLink(v);
    });
  }
  function bannerModel() {
    if (!S.booted) return null;
    const remote = S.conn.mode === 'remote';
    const name = computerName();
    if (S.link === 'unpaired')
      return {
        kind: 'bad',
        title: `${ThisDevice()} isn’t paired with ${name} anymore`,
        body: 'It may have been removed from that computer’s paired devices. Pair again with a new 6-digit code.',
        actions: [
          ['Pair again', () => openSetupWizard({ step: 2, url: S.conn.url }), 'primary'],
          [`Use ${thisDevice()} instead`, () => useThisDevice()],
        ],
      };
    if (S.link === 'unreachable' || S.link === 'retrying')
      return {
        kind: 'bad',
        title: remote ? `Can’t reach ${name}` : 'Can’t reach openbot',
        list: remote ? [`Is ${name} turned on and awake?`, `Is Tailscale on, on ${thisDevice()} and on ${name}?`] : ['Is the openbot app still running?'],
        note: 'Trying again automatically…',
        actions: [
          [S.link === 'retrying' ? 'Retrying…' : 'Retry', retryNow, 'primary', S.link === 'retrying'],
          remote && [`Use ${thisDevice()} instead`, () => useThisDevice()],
          remote && ['Help', () => openSetupWizard({ step: 1 })],
        ],
      };
    if (S.link === 'connecting') return { kind: 'wait', title: 'Connecting to your computer…' };
    const c = S.computer && S.computer.claude;
    if (c && !c.installed)
      return { kind: 'warn', title: `Claude Code isn’t installed on ${name}`, body: 'Your Bots need it to work.', actions: [['How to fix', openClaudeHelp, 'primary'], ['Check again', recheckClaude]] };
    if (c && !c.logged_in)
      return { kind: 'warn', title: `Claude isn’t signed in on ${name}`, body: `Open ${shellName()} there, type claude, and sign in.`, actions: [['How to fix', openClaudeHelp, 'primary'], ['Check again', recheckClaude]] };
    return null;
  }
  function renderSysBanner() {
    const el = $('#sbBanner');
    const pill = $('#connBanner');
    if (!el) return;
    const m = bannerModel();
    el.textContent = '';
    el.hidden = !m;
    el.className = 'sb-banner' + (m ? ' ' + m.kind : '');
    if (m) {
      appendKids(el, [
        h('div', { class: 'sbb-head' }, m.kind === 'wait' ? spinner('xs') : icon('alert', 15, '', 2), h('span', { class: 'sbb-title' }, m.title)),
        m.body && h('div', { class: 'sbb-body' }, m.body),
        m.list && h('ul', { class: 'sbb-list' }, m.list.map((t) => h('li', null, t))),
        m.actions && h('div', { class: 'sbb-actions' }, m.actions.filter(Boolean).map(([l, fn, cls, dis]) => h('button', { class: 'btn small ' + (cls || ''), disabled: !!dis, onclick: fn }, l))),
        m.note && h('div', { class: 'sbb-note' }, m.note),
      ]);
    }
    // compact pill at the top of the chat when the sidebar is narrow or hidden
    const narrow = app.classList.contains('rail') || innerWidth <= 520;
    if (m && m.kind === 'bad') {
      clearTimeout(S.bannerTimer);
      pill.textContent = '';
      pill.append(icon('alert', 13, '', 2), h('span', null, m.title));
      if (narrow) pill.append(h('button', { class: 'pill-retry', onclick: retryNow }, 'Retry'));
      pill.classList.add('bad');
      pill.hidden = !narrow;
    } else if (pill.classList.contains('bad')) {
      pill.classList.remove('bad');
      pill.hidden = true;
    }
  }

  // ───────────────────────────── global wiring ─────────────────────────────
  $('#searchBtn').append(icon('search', 20));
  $('#newBotBtn').append(icon('plus', 20));
  $('#railSearchBtn').append(icon('search', 20));
  $('#railNewBtn').append(icon('plus', 20));
  $('#templatesBtn').append(icon('gridPlus', 20));
  $('#settingsBtn').append(icon('gear', 19));
  $('#menuBtn').append(icon('menu', 18));
  $('#screenBtn').append(icon('monitor', 18), h('span', { class: 'bs-dot', 'aria-hidden': 'true' }));
  $('#screenBtn').addEventListener('click', () => {
    const b = curBot();
    if (b) openScreen(b.id);
  });
  $('#attachBtn').append(icon('plus', 16, '', 2.2));
  $('#sendBtn').append(icon('arrowUp', 16, 'g-send', 2.4), icon('stop', 22, 'g-stop'));
  $('#toBottom').append(icon('arrowDown', 18, '', 2));
  $('#searchBtn').dataset.kbd = MOD + 'K';
  $('#newBotBtn').dataset.kbd = MOD + 'N';
  $('#railSearchBtn').dataset.kbd = MOD + 'K';
  $('#railNewBtn').dataset.kbd = MOD + 'N';
  $('#settingsBtn').dataset.kbd = MOD + ',';

  $('#searchBtn').addEventListener('click', () => S.booted && openSearch());
  $('#railSearchBtn').addEventListener('click', () => S.booted && openSearch());
  // typing a letter on the focused Search button opens the palette with it
  $('#searchBtn').addEventListener('keydown', (e) => {
    if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey && e.key !== ' ') {
      e.preventDefault();
      openSearch(e.key);
    }
  });
  $('#newBotBtn').addEventListener('click', (e) => S.booted && newMenu(e.currentTarget));
  $('#railNewBtn').addEventListener('click', (e) => S.booted && newMenu(e.currentTarget));
  $('#templatesBtn').addEventListener('click', () => S.booted && openTemplates());
  $('#settingsBtn').addEventListener('click', () => openSettings('general'));
  $('#meBtn').addEventListener('click', () => accountMenu());
  $('#island').addEventListener('click', () => setPanel(!S.panelOpen));
  $('#menuBtn').addEventListener('click', () => app.classList.toggle('sidebar-open'));
  $('#scrim').addEventListener('click', () => app.classList.remove('sidebar-open'));
  $('#toBottom').addEventListener('click', () => scrollBottom(true));
  $('#newPill').addEventListener('click', () => scrollBottom(true));

  // hold ⌘ for half a second to see ⌘1…⌘9 on the sidebar rows
  let kbdTimer = null;
  const hideKbd = () => {
    clearTimeout(kbdTimer);
    kbdTimer = null;
    $('#sidebar').classList.remove('show-kbd');
  };
  window.addEventListener('blur', hideKbd);
  document.addEventListener('keyup', (e) => {
    if (e.key === 'Meta' || e.key === 'Control') hideKbd();
  });

  document.addEventListener('keydown', (e) => {
    if ((isMac ? e.key === 'Meta' : e.key === 'Control') && !kbdTimer) {
      kbdTimer = setTimeout(() => $('#sidebar').classList.add('show-kbd'), 500);
    } else if (kbdTimer && !(e.key === 'Meta' || e.key === 'Control')) hideKbd();

    if (e.key === 'Escape') {
      if (openMenuEl) return closeMenu();
      const top = modalStack[modalStack.length - 1];
      if (top && top.dismissable) {
        e.preventDefault();
        top.close();
        return;
      }
      if (S.panelOpen && !(document.activeElement && panelEl.contains(document.activeElement) && isEditable(document.activeElement))) {
        setPanel(false);
        return;
      }
      if (app.classList.contains('sidebar-open')) app.classList.remove('sidebar-open');
      return;
    }
    if (!S.booted) return;
    const mod = modKey(e);
    const k = e.key.toLowerCase();
    if (mod && !e.altKey) {
      if (k === 'k' && !e.shiftKey) {
        e.preventDefault();
        if (S.paletteClose) S.paletteClose();
        else if (!modalStack.length) openSearch();
        return;
      }
      if (k === 'f' && e.shiftKey) {
        e.preventDefault();
        if (!modalStack.length) openSearch();
        return;
      }
      if (modalStack.length) return;
      if ((e.code === 'Comma' || k === ',' || k === '<') && e.shiftKey) {
        e.preventDefault();
        setPanel(!S.panelOpen);
      } else if (k === ',') {
        e.preventDefault();
        openSettings();
      } else if (k === 'n' && !e.shiftKey) {
        e.preventDefault();
        openNewBot();
      } else if (k === 'n' && e.shiftKey) {
        e.preventDefault();
        openNewGroup();
      } else if (k === 'b' && !e.shiftKey) {
        e.preventDefault();
        toggleRail();
      } else if (k === 'i' && !e.shiftKey) {
        e.preventDefault();
        input.focus();
      } else if (k === '/' || e.code === 'Slash') {
        e.preventDefault();
        openShortcuts();
      } else if (/^[1-9]$/.test(e.key) && !e.shiftKey) {
        e.preventDefault();
        const b = orderedBots()[Number(e.key) - 1];
        if (b) selectBot(b.id);
      }
      return;
    }
    if (e.altKey && !mod && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      if (modalStack.length) return;
      e.preventDefault();
      const order = orderedBots();
      const i = order.findIndex((b) => b.id === S.current);
      const next = order[clamp(i + (e.key === 'ArrowDown' ? 1 : -1), 0, order.length - 1)];
      if (next) selectBot(next.id);
      return;
    }
    // quick replies: A, B, C… pick an option of the open question card
    if (!modalStack.length && !isEditable(document.activeElement) && !e.ctrlKey && !e.metaKey && !e.altKey && /^[a-z]$/i.test(e.key) && !input.value) {
      const b = curBot();
      const w = b && openWidget(S.msgs.get(b.id) || []);
      const c = w && S.nodes.get(w.id);
      const card = c && $('.card.widget', c.el);
      const i = e.key.toUpperCase().charCodeAt(0) - 65;
      if (card && card._opts && i < card._opts.length) {
        e.preventDefault();
        card._answer(card._opts[i]);
        return;
      }
    }
    // type-to-focus: printable keys go to the composer
    if (!modalStack.length && !isEditable(document.activeElement) && !e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1 && curBot() && !input.disabled) {
      if (e.key === ' ' && document.activeElement && document.activeElement.tagName === 'BUTTON') return;
      input.focus();
    }
  });

  // refresh relative things
  setInterval(() => {
    if (S.settingsUI && (S.settingsUI.tab === 'usage' || S.settingsUI.tab === 'computer')) rerenderSettings();
  }, 60000);

  // ───────────────────────────── phone notifications (sw.js, push.rs) ─────────────────────────────
  // Web Push needs the service worker. On an iPhone it only works once openbot is on the Home Screen.
  const pushPossible = () => !TAURI && window.isSecureContext && 'serviceWorker' in navigator && 'PushManager' in window && 'Notification' in window;
  const isIOS = () => /iPhone|iPad/.test(navigator.userAgent) || (/Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1);
  const standalone = () => navigator.standalone === true || matchMedia('(display-mode: standalone)').matches;
  let swReg = null;
  async function registerSW() {
    if (TAURI || !window.isSecureContext || !('serviceWorker' in navigator)) return null;
    try {
      swReg = await navigator.serviceWorker.register('/sw.js', { scope: '/' });
    } catch {
      swReg = null;
    }
    return swReg;
  }
  if (!TAURI && 'serviceWorker' in navigator) {
    // A notification was tapped while openbot was open: show that chat.
    navigator.serviceWorker.addEventListener('message', (e) => {
      const bot = e.data && e.data.type === 'open-bot' && e.data.bot;
      if (bot && S.bots.has(bot)) selectBot(bot, { focus: true });
    });
  }
  function b64urlToBytes(s) {
    const b = atob(s.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (s.length % 4)) % 4));
    return Uint8Array.from(b, (c) => c.charCodeAt(0));
  }
  async function currentPushSub() {
    const reg = swReg || (await registerSW());
    return reg ? reg.pushManager.getSubscription() : null;
  }
  // Must run straight from a tap: iPhones only ask for permission then.
  async function enablePush() {
    const perm = await Notification.requestPermission();
    if (perm !== 'granted') throw new Error(perm === 'denied' ? 'Notifications are blocked for openbot. Allow them in this phone’s Settings › Notifications › openbot.' : 'Notifications weren’t allowed.');
    const reg = swReg || (await registerSW());
    if (!reg) throw new Error('This browser can’t show notifications from openbot.');
    const { key } = await api('/api/push/key');
    let sub = await reg.pushManager.getSubscription();
    if (sub) {
      // Subscribed for another computer's key: start over.
      const k = sub.options && sub.options.applicationServerKey;
      const same = k && btoa(String.fromCharCode(...new Uint8Array(k))) === btoa(String.fromCharCode(...b64urlToBytes(key)));
      if (!same) {
        await sub.unsubscribe().catch(() => {});
        sub = null;
      }
    }
    if (!sub) sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: b64urlToBytes(key) });
    await api('/api/push/subscribe', { method: 'POST', body: sub.toJSON() });
    return sub;
  }
  async function disablePush() {
    const sub = await currentPushSub();
    if (!sub) return;
    await api('/api/push/unsubscribe', { method: 'POST', body: { endpoint: sub.endpoint } }).catch(() => {});
    await sub.unsubscribe().catch(() => {});
  }
  // Settings › General › Notifications (in a browser; the desktop app has its own).
  function pushSection() {
    if (TAURI) return null;
    const st = (S.pushUI = S.pushUI || { sub: undefined, busy: false, msg: '' });
    if (st.sub === undefined && pushPossible()) {
      st.sub = null;
      currentPushSub().then((sub) => {
        st.sub = sub || null;
        rerenderSettings(true);
      });
    }
    let body;
    if (isIOS() && !standalone()) {
      body = row('Notifications on this phone', null, 'Add openbot to your Home Screen first: tap Share, then Add to Home Screen. Open openbot from there and turn this on.');
    } else if (!pushPossible()) {
      body = row('Notifications on this device', null, window.isSecureContext ? 'This browser can’t show notifications from openbot.' : 'Open openbot at its https:// Tailscale address to get notifications.');
    } else {
      const on = !!st.sub;
      const toggle = toggleEl(on, async (v) => {
        st.busy = true;
        st.msg = '';
        try {
          if (v) st.sub = await enablePush();
          else {
            await disablePush();
            st.sub = null;
          }
        } catch (e) {
          st.msg = e.message;
          toggle.checked = !v;
        }
        st.busy = false;
        rerenderSettings(true);
      });
      toggle.disabled = st.busy;
      const test = async () => {
        st.msg = 'Sending…';
        rerenderSettings(true);
        try {
          await api('/api/push/test', { method: 'POST', body: { endpoint: st.sub.endpoint } });
          st.msg = 'Sent. It should arrive in a few seconds.';
        } catch (e) {
          st.msg = e.message;
          if (e.status === 502 && /turned off/.test(e.message)) st.sub = null;
        }
        rerenderSettings(true);
      };
      body = [
        row('Notifications on this device', toggle, st.msg || 'When a Bot finishes or needs you while openbot isn’t open anywhere. Bots you’ve muted stay quiet.'),
        on ? row('Test', pillBtn('Send a Test', test), 'Check that notifications reach this device') : null,
      ];
    }
    return section('Notifications', card(body));
  }

  // ───────────────────────────── start ─────────────────────────────
  initToken();
  registerSW();
  loadDraftStore();
  applyTheme();
  layoutShell();
  renderMe();
  updateComposer();
  renderAttachRow();
  boot();
})();
