// openbot's service worker, for phones that added openbot to their home screen: it shows the
// notifications the computer sends (Web Push) and opens the right chat when one is tapped.
// It caches nothing, so openbot always loads fresh from the computer.

self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

self.addEventListener('push', (e) => {
  let d = {};
  try {
    d = e.data ? e.data.json() : {};
  } catch {
    d = { body: e.data ? e.data.text() : '' };
  }
  e.waitUntil(
    (async () => {
      // The number on the app icon: unread messages.
      if (typeof d.badge === 'number' && self.navigator.setAppBadge) {
        try {
          if (d.badge > 0) await self.navigator.setAppBadge(d.badge);
          else await self.navigator.clearAppBadge();
        } catch {}
      }
      // Always show something: iPhones stop delivering to web apps that receive pushes silently.
      await self.registration.showNotification(d.title || 'openbot', {
        body: d.body || '',
        tag: d.tag || undefined,
        icon: '/icons/phone-192.png',
        data: { bot: d.bot || '' },
      });
    })()
  );
});

self.addEventListener('notificationclick', (e) => {
  e.notification.close();
  const bot = (e.notification.data && e.notification.data.bot) || '';
  e.waitUntil(
    (async () => {
      const wins = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
      for (const w of wins) {
        w.postMessage({ type: 'open-bot', bot });
        if ('focus' in w) return w.focus();
      }
      return self.clients.openWindow(bot ? '/#bot=' + encodeURIComponent(bot) : '/');
    })()
  );
});
