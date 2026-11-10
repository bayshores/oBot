// Controls the browser on a Bot's screen through its local debugging port: which page is in front,
// filling boxes (saved logins, secure forms), opening pages and managing tabs.
//
// screen.rs runs this inside Linux with `node -e`. The browser writes its port to
// DevToolsActivePort in its profile (it's started with --remote-debugging-port=0, local only).
// Input (environment):
//   OB_BOT  the Bot's id (its browser profile is ~/.obot/screens/<id>/profile)
//   OB_REQ  JSON, one of:
//     {mode: "inspect"}                          -> {url, title}
//     {mode: "fill", host, fields: [{target, value}]} -> {url, filled: [targets], missing: [targets]}
//         target: "username" | "password" | "new_password" | "one_time_code" | "focused" | a CSS selector
//     {mode: "navigate", url}                    -> {url}   (in the tab in front)
//     {mode: "images"}                           -> {url, images: [{src, alt, w, h, link}]}  pictures on the page
//     {mode: "tabs"}                             -> {tabs: [{id, title, url, front}]}  most recently used first
//     {mode: "switch", id} / {mode: "close", id} -> {tabs}
//     {mode: "close_others"}                     -> {tabs}
//     {mode: "trim", keep}                       -> {tabs}   close the oldest tabs beyond `keep` (blank ones first)
// Output: one JSON line, or {error: "sentence for the Bot"}. Values are only typed into the page, never printed.
'use strict';
const fs = require('fs');
const os = require('os');
const { execFileSync } = require('child_process');

const done = (o) => {
  process.stdout.write(JSON.stringify(o) + '\n');
  process.exit(0);
};
const NOT_OPEN = "The browser on your screen isn't open. Open the page with computer_open first.";

// A minimal client for the browser's debugging protocol.
function connect(url) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    const waiting = new Map();
    let next = 0;
    const timer = setTimeout(() => reject(new Error('timed out')), 5000);
    ws.onerror = () => {
      clearTimeout(timer);
      reject(new Error('no connection'));
    };
    ws.onmessage = (e) => {
      const m = JSON.parse(e.data);
      const w = m.id && waiting.get(m.id);
      if (!w) return;
      waiting.delete(m.id);
      if (m.error) w.reject(new Error(m.error.message));
      else w.resolve(m.result);
    };
    ws.onopen = () => {
      clearTimeout(timer);
      resolve({
        send(method, params = {}) {
          return new Promise((res, rej) => {
            const id = ++next;
            waiting.set(id, { resolve: res, reject: rej });
            ws.send(JSON.stringify({ id, method, params }));
            setTimeout(() => waiting.delete(id) && rej(new Error('timed out')), 8000);
          });
        },
        close() {
          try {
            ws.close();
          } catch {}
        },
      });
    };
  });
}

async function evaluate(c, expression) {
  const r = await c.send('Runtime.evaluate', { expression, returnByValue: true });
  if (r.exceptionDetails) throw new Error('the page script failed');
  return r.result.value;
}

// Runs inside the page: finds the box for `target`, focuses it and selects what's in it (so the
// typed value replaces it). Returns false if there's no such box.
const FIND = function (target) {
  const shown = (e) => {
    if (!e || e.disabled || e.readOnly) return false;
    const r = e.getBoundingClientRect();
    const s = getComputedStyle(e);
    return r.width > 2 && r.height > 2 && s.visibility !== 'hidden' && s.display !== 'none' && s.opacity !== '0';
  };
  const words = (e) => [e.name, e.id, e.placeholder, e.getAttribute('aria-label'), e.autocomplete].join(' ').toLowerCase();
  const inputs = [...document.querySelectorAll('input')].filter(shown);
  const texty = inputs.filter((e) => ['text', 'email', 'tel', 'number', 'url'].includes(e.type));
  const passwords = inputs.filter((e) => e.type === 'password');
  let el = null;
  if (target === 'password') el = passwords.find((e) => e.autocomplete !== 'new-password') || passwords[0];
  else if (target === 'new_password') el = passwords.find((e) => e.autocomplete === 'new-password') || passwords[passwords.length - 1];
  else if (target === 'username') {
    const pw = passwords[0];
    const before = pw ? texty.filter((e) => e.compareDocumentPosition(pw) & Node.DOCUMENT_POSITION_FOLLOWING) : [];
    el =
      texty.find((e) => /\b(username|email)\b/.test(e.autocomplete)) ||
      before[before.length - 1] ||
      texty.find((e) => /user|email|e-mail|login|account|ident/.test(words(e))) ||
      (texty.length === 1 ? texty[0] : null);
  } else if (target === 'one_time_code') {
    el =
      inputs.find((e) => e.autocomplete === 'one-time-code') ||
      texty.find((e) => /otp|code|token|passcode|2fa|mfa|verif|\bpin\b/.test(words(e))) ||
      (texty.length === 1 ? texty[0] : null);
  } else if (target === 'focused') {
    const a = document.activeElement;
    el = a && (a.tagName === 'INPUT' || a.tagName === 'TEXTAREA') && shown(a) ? a : null;
  } else {
    try {
      el = document.querySelector(target);
    } catch (e) {}
    if (!shown(el)) el = null;
  }
  if (!el) return false;
  el.scrollIntoView({ block: 'center' });
  el.focus();
  try {
    el.select();
  } catch (e) {}
  if (el.value && el.selectionStart === el.selectionEnd) {
    el.value = '';
    el.dispatchEvent(new Event('input', { bubbles: true }));
  }
  return document.activeElement === el;
};

// Runs inside the page: its preview picture, then the pictures big enough to matter, top to bottom.
const IMAGES = function () {
  const out = [];
  const seen = new Set();
  const add = (src, alt, w, h, link) => {
    if (!/^https?:/.test(src || '') || seen.has(src) || out.length >= 40) return;
    seen.add(src);
    out.push({ src, alt: (alt || '').replace(/\s+/g, ' ').trim().slice(0, 140), w, h, link: /^https?:/.test(link || '') ? link : '' });
  };
  const og = document.querySelector('meta[property="og:image"], meta[name="twitter:image"]');
  if (og) add(new URL(og.content, location.href).href, 'Preview picture of this page', 0, 0, '');
  for (const img of document.images) {
    const r = img.getBoundingClientRect();
    if (r.width < 100 || r.height < 100) continue;
    const a = img.closest('a');
    add(img.currentSrc || img.src, img.alt || img.title || (a && a.textContent), Math.round(r.width), Math.round(r.height), a && a.href);
  }
  return out;
};

const short = (id) => id.slice(0, 6).toLowerCase();
const BLANK = /^(about:blank|chrome:\/\/newtab|chrome:\/\/new-tab-page)/;

(async () => {
  if (typeof WebSocket !== 'function') done({ error: 'Controlling the browser needs Node.js 22 or newer inside Linux.' });
  const req = JSON.parse(process.env.OB_REQ || '{}');
  const portFile = `${os.homedir()}/.obot/screens/${process.env.OB_BOT}/profile/DevToolsActivePort`;
  const http = (port, path, method = 'GET') => fetch(`http://127.0.0.1:${port}${path}`, { method });
  // A browser that just started may still be writing its port or restoring tabs: wait a little.
  let port = '';
  let pages = null;
  for (let i = 0; i < (req.mode === 'trim' ? 10 : 1); i++) {
    try {
      port = fs.readFileSync(portFile, 'utf8').split('\n')[0].trim();
      pages = await (await http(port, '/json/list')).json();
      break;
    } catch {
      await new Promise((r) => setTimeout(r, 1000));
    }
  }
  if (!pages) done({ error: NOT_OPEN });
  pages = pages.filter((p) => p.type === 'page' && p.webSocketDebuggerUrl);

  // The tab in front is the visible one. With several browser windows, the window title decides.
  let front = [];
  for (const p of pages) {
    try {
      const c = await connect(p.webSocketDebuggerUrl);
      if ((await evaluate(c, 'document.visibilityState')) === 'visible') front.push({ p, c });
      else c.close();
    } catch {}
  }
  if (front.length > 1) {
    let win = '';
    try {
      win = execFileSync('xdotool', ['getactivewindow', 'getwindowname'], { encoding: 'utf8', timeout: 3000 }).trim();
    } catch {}
    front = front.filter(({ p }) => p.title && (win === p.title || win.startsWith(p.title + ' - ')));
    // Longest title wins, so a "Login" tab doesn't match a window showing "Login - Portal".
    const longest = Math.max(0, ...front.map(({ p }) => p.title.length));
    front = front.filter(({ p }) => p.title.length === longest);
  }
  const frontId = front.length === 1 ? front[0].p.id : null;

  // Tab management.
  const list = async (fid = frontId) => {
    await new Promise((r) => setTimeout(r, 300));
    const now = (await (await http(port, '/json/list')).json()).filter((p) => p.type === 'page');
    if (!now.some((p) => p.id === fid)) fid = now[0] && now[0].id;
    return now.map((p) => ({ id: short(p.id), title: p.title, url: p.url, front: p.id === fid }));
  };
  const byShort = (id) => pages.find((p) => short(p.id) === String(id || '').toLowerCase());
  if (req.mode === 'tabs') done({ tabs: await list() });
  if (req.mode === 'switch' || req.mode === 'close') {
    const t = byShort(req.id);
    if (!t) done({ error: `No tab with id ${req.id}. List the tabs again.` });
    if (req.mode === 'close' && pages.length === 1) done({ error: "That's the only tab; open another page instead of closing it." });
    await http(port, `/json/${req.mode === 'switch' ? 'activate' : 'close'}/${t.id}`);
    done({ tabs: await list(req.mode === 'switch' ? t.id : frontId) });
  }
  if (req.mode === 'close_others' || req.mode === 'trim') {
    let keepId = frontId || (pages[0] && pages[0].id);
    // A blank tab in front (a freshly started browser) gives way to the last real page.
    const realTab = pages.find((p) => !BLANK.test(p.url));
    const keepPage = pages.find((p) => p.id === keepId);
    if (req.mode === 'trim' && keepPage && BLANK.test(keepPage.url) && realTab) {
      keepId = realTab.id;
      await http(port, `/json/activate/${keepId}`);
    }
    let extra = pages.filter((p) => p.id !== keepId);
    if (req.mode === 'trim') {
      // Blank tabs go first, then the least recently used ones.
      const keep = Math.max(1, Number(req.keep) || 1) - 1;
      const real = extra.filter((p) => !BLANK.test(p.url));
      extra = [...extra.filter((p) => BLANK.test(p.url)), ...real.slice(keep)];
    }
    for (const p of extra) await http(port, `/json/close/${p.id}`);
    done({ tabs: await list(keepId) });
  }

  if (!frontId) done({ error: "Can't tell which page is in front on your screen. Close extra browser windows or click on the page, then try again." });
  const { p, c } = front[0];
  if (req.mode === 'navigate') {
    await c.send('Page.navigate', { url: req.url });
    await new Promise((r) => setTimeout(r, 1500));
    done({ url: req.url });
  }
  const url = await evaluate(c, 'location.href');
  if (req.mode === 'inspect') done({ url, title: p.title });
  if (req.mode === 'images') done({ url, images: await evaluate(c, `(${IMAGES})()`) });

  // Only fill the page oBot checked: same https website as when it decided.
  let host = '';
  try {
    const u = new URL(url);
    host = u.protocol === 'https:' ? u.hostname : '';
  } catch {}
  if (!host || host !== req.host) done({ error: `The page in front changed (it's now ${url}), so nothing was filled.` });
  const filled = [];
  const missing = [];
  for (const f of req.fields || []) {
    if (!(await evaluate(c, `(${FIND})(${JSON.stringify(f.target)})`))) {
      missing.push(f.target);
      continue;
    }
    await c.send('Input.insertText', { text: f.value });
    filled.push(f.target);
  }
  done({ url, filled, missing });
})().catch((e) => done({ error: `Couldn't reach the browser on your screen (${e.message}).` }));
