// Runs inside every page a Bot shows in the chat (the server adds it; see backend/core/src/pages.rs).
// It draws the icons (<i data-icon="name"></i>) and tells the chat how tall the page is.
// OBOT_ICONS (name -> drawing) is put in front of this file by the server.
(function () {
  var icons = window.OBOT_ICONS || {};
  function draw(el) {
    var body = icons[el.getAttribute('data-icon')];
    if (!body || el.firstChild) return;
    el.setAttribute('aria-hidden', 'true');
    el.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">' + body + '</svg>';
  }
  function drawAll(root) {
    if (root.nodeType !== 1) return;
    if (root.matches('[data-icon]')) draw(root);
    root.querySelectorAll('[data-icon]').forEach(draw);
  }
  drawAll(document.documentElement);
  // Icons a page's own script adds later.
  new MutationObserver(function (list) {
    list.forEach(function (m) {
      m.addedNodes.forEach(drawAll);
    });
  }).observe(document.documentElement, { childList: true, subtree: true });

  function tell() {
    try {
      parent.postMessage({ type: 'obot:height', height: document.documentElement.scrollHeight }, '*');
    } catch (e) {}
  }
  addEventListener('load', tell);
  addEventListener('resize', tell);
  if (window.ResizeObserver) new ResizeObserver(tell).observe(document.documentElement);
  setTimeout(tell, 0);
})();
