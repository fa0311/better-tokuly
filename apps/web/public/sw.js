/*
 * Tokuly service worker. Plain JavaScript, no imports, no build step.
 *
 *   1. Lifecycle             take over as soon as a new version is installed
 *   2. Push                  show the notification the server sent
 *   3. Notification click    focus or open the app at the notification's path
 *   4. Subscription change   tell open pages to register the new subscription
 *
 * There is no `fetch` handler: every request goes straight to the network.
 * Offline support would add one as its own section, with its own cache names,
 * and leave `/api/`, `/media/` and `/tiles/` alone.
 *
 * Messages to pages (received on `navigator.serviceWorker`):
 *   { type: "tokuly:notification-click", id, path }   answered through the message port
 *   { type: "tokuly:push-subscription-change" }
 */

const APP_ICON = "/icon.svg";
/** Where a click goes when no open page takes it: resolves `path` inside the app and marks `id` read. */
const OPEN_ROUTE = "/notifications/open";
const INBOX_PATH = "/notifications";

// ── 1. Lifecycle ────────────────────────────────────────────────────

self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

// ── 2. Push ─────────────────────────────────────────────────────────

/*
 * The payload is a declarative push message (docs/events/notify.md):
 *   { web_push: 8030,
 *     notification: { title, body, lang, navigate, tag, image, timestamp, requireInteraction,
 *                     data: { id, kind, category, path, extra, urgent } },
 *     app_badge }
 */
self.addEventListener("push", (event) => {
  event.waitUntil(showPush(event.data));
});

async function showPush(data) {
  let message = null;
  try {
    message = data ? data.json() : null;
  } catch {
    // Not JSON: fall through to the generic notification.
  }
  const notification = message && typeof message.notification === "object" ? message.notification : null;

  // A push must always end in a visible notification, even one we cannot read.
  const title = notification && typeof notification.title === "string" && notification.title ? notification.title : "Tokuly";
  const details = notification || {};
  const options = {
    body: typeof details.body === "string" ? details.body : "",
    icon: APP_ICON,
    data: { ...(details.data || {}), url: details.navigate || "" },
  };
  if (details.lang) options.lang = details.lang;
  if (details.image) options.image = details.image;
  if (details.timestamp) options.timestamp = details.timestamp;
  if (details.requireInteraction) options.requireInteraction = true;
  if (details.tag) {
    // Same tag = the newer notification replaces the older one, and announces itself again.
    options.tag = details.tag;
    options.renotify = true;
  }

  await self.registration.showNotification(title, options);
  await setAppBadge(message ? message.app_badge : undefined);
}

/** The unread count on the installed app's icon, where the platform has one. */
async function setAppBadge(count) {
  if (typeof count !== "number" || !self.navigator.setAppBadge) return;
  try {
    if (count > 0) await self.navigator.setAppBadge(count);
    else await self.navigator.clearAppBadge();
  } catch {
    // Cosmetic.
  }
}

// ── 3. Notification click ───────────────────────────────────────────

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(openTarget(event.notification.data || {}));
});

async function openTarget(data) {
  const click = {
    type: "tokuly:notification-click",
    id: typeof data.id === "string" ? data.id : "",
    path: typeof data.path === "string" && data.path ? data.path : INBOX_PATH,
  };
  const fallback = new URL(OPEN_ROUTE, self.location.origin);
  fallback.searchParams.set("path", click.path);
  if (click.id) fallback.searchParams.set("id", click.id);

  const pages = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
  const page = pages.find((client) => client.focused) || pages.find((client) => client.visibilityState === "visible") || pages[0];
  if (!page) return self.clients.openWindow(fallback.href);

  const focused = await page.focus().catch(() => page);
  // A page with the app running navigates by itself, without reloading. Any
  // other page (signed out, still loading) does not answer and is sent there.
  if (await delivered(focused, click)) return;
  await focused.navigate(fallback.href).catch(() => self.clients.openWindow(fallback.href));
}

/** Posts `message` to a page and resolves to whether the page acknowledged it. */
function delivered(client, message, timeoutMs = 1500) {
  return new Promise((resolve) => {
    const channel = new MessageChannel();
    const timer = setTimeout(() => resolve(false), timeoutMs);
    channel.port1.onmessage = () => {
      clearTimeout(timer);
      resolve(true);
    };
    client.postMessage(message, [channel.port2]);
  });
}

// ── 4. Subscription change ──────────────────────────────────────────

// The browser replaced or dropped the push subscription. Registering it needs
// the signed-in session, which only pages have; a closed app catches up on its
// next start.
self.addEventListener("pushsubscriptionchange", (event) => {
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((pages) => {
      for (const page of pages) page.postMessage({ type: "tokuly:push-subscription-change" });
    }),
  );
});
