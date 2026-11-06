// Fills boxes on the page in front of a Bot's screen (saved logins, secure forms).
//
// screen.rs runs this inside Linux with `node -e`, talking to the screen's browser through its
// local debugging port (the browser writes the port to DevToolsActivePort in its profile).
// Input (environment):
//   OB_BOT  the Bot's id (its browser profile is ~/.openbot/screens/<id>/profile)
//   OB_REQ  JSON {mode: "inspect"} or {mode: "fill", host, fields: [{target, value}]}
//           target: "username" | "password" | "new_password" | "one_time_code" | "focused" | a CSS selector
// Output: one JSON line. inspect -> {url, title}; fill -> {url, filled: [targets], missing: [targets]};
// or {error: "sentence for the Bot"}. Values are only typed into the page, never printed.
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

(async () => {
  if (typeof WebSocket !== 'function') done({ error: 'Filling pages needs Node.js 22 or newer inside Linux.' });
  const req = JSON.parse(process.env.OB_REQ || '{}');
  let port = '';
  try {
    port = fs.readFileSync(`${os.homedir()}/.openbot/screens/${process.env.OB_BOT}/profile/DevToolsActivePort`, 'utf8').split('\n')[0].trim();
  } catch {}
  if (!/^\d+$/.test(port)) done({ error: NOT_OPEN });
  let pages = [];
  try {
    pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  } catch {
    done({ error: NOT_OPEN });
  }
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
  if (front.length !== 1) done({ error: "Can't tell which page is in front on your screen. Close extra browser windows or click on the page, then try again." });
  const { p, c } = front[0];
  const url = await evaluate(c, 'location.href');
  if (req.mode === 'inspect') done({ url, title: p.title });

  // Only fill the page openbot checked: same https website as when it decided.
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
})().catch((e) => done({ error: `Couldn't reach the page on your screen (${e.message}).` }));
