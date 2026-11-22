/*
 * Tokuly service worker. Plain JavaScript, no imports, no build step.
 *
 *   1. Lifecycle             a new version waits until the person says "update"
 *   2. Offline               caches for static files, the offline page for navigations
 *   3. Push                  show the notification the server sent
 *   4. Notification click    focus or open the app at the notification's path
 *   5. Subscription change   tell open pages to register the new subscription
 *   6. Messages from pages   update now, clear the offline data
 *
 * Messages to pages (received on `navigator.serviceWorker`):
 *   { type: "tokuly:notification-click", id, path }   answered through the message port
 *   { type: "tokuly:push-subscription-change" }
 *
 * Messages from pages (`postMessage` on the worker):
 *   { type: "tokuly:skip-waiting" }          sent to a waiting worker: take over now
 *   { type: "tokuly:clear-offline-data" }    answered through the message port with { ok }
 */

const APP_ICON = "/icons/icon-192.png";
/** The small single-colour image next to a notification in the status bar (Android). */
const APP_BADGE = "/icons/badge-96.png";
/** Where a click goes when no open page takes it: resolves `path` inside the app and marks `id` read. */
const OPEN_ROUTE = "/notifications/open";
const INBOX_PATH = "/notifications";

// ── 1. Lifecycle ────────────────────────────────────────────────────

/*
 * No `skipWaiting` here. The first worker of a browser activates by itself
 * and takes the open pages (`claim`). A later version stays "waiting" while a
 * page of the old one is open: the app shows "新しいバージョンがあるよ" and sends
 * `tokuly:skip-waiting` when the person taps it (section 6), so nobody's page
 * changes its worker underneath them.
 */
self.addEventListener("install", (event) => {
  // Not fatal: push must work even where the offline page cannot be fetched right now.
  event.waitUntil(storeOfflinePage().catch(() => {}));
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      await Promise.allSettled([dropForeignCaches(), enableNavigationPreload(), storeOfflinePage(), ...RUNTIME_CACHES.map(trim)]);
      await self.clients.claim();
    })(),
  );
});

// ── 2. Offline ──────────────────────────────────────────────────────

/*
 * What is kept, and only this:
 *
 *   tokuly-shell    /offline.html, stored when the worker installs
 *   tokuly-assets   /assets/*: the app's scripts, styles and fonts. Their names
 *                   carry a content hash, so a stored file never goes stale:
 *                   cache first.
 *   tokuly-map      /map-assets/*: map fonts and sprites. Answered from the
 *                   cache while a fresh copy is fetched for next time.
 *
 * Navigations go to the network; `/offline.html` answers when it cannot be
 * reached. Everything else is not touched at all: `/api/`, `/media/` and
 * `/tiles/`, other origins, anything that is not a GET, and every path that
 * is not listed above. That includes all modules of the development server
 * (`/src/`, `/@vite/`, `/node_modules/`), so development never sees a cached
 * module. Nothing is stored by name at install time except the offline page:
 * what a build needs is learned as it is used.
 *
 * Cache names start with `tokuly-` (the app's "clear offline data" relies on
 * it); bump the version when what is stored changes shape.
 */
const CACHE_PREFIX = "tokuly-";
const SHELL_CACHE = `${CACHE_PREFIX}shell-v1`;
const OFFLINE_URL = "/offline.html";
const DAY_MS = 24 * 60 * 60_000;

const RUNTIME_CACHES = [
  {
    cache: `${CACHE_PREFIX}assets-v1`,
    // `name-<hash>.ext`, as the build writes it. Anything else under /assets/ is not known to be immutable.
    matches: (url) => url.pathname.startsWith("/assets/") && /-[\w-]{8,}\.[a-z0-9]+$/i.test(url.pathname),
    strategy: cacheFirst,
    // A build is around 150 files, most of them font slices; this holds a few builds.
    maxEntries: 500,
    maxAgeMs: 45 * DAY_MS,
  },
  {
    cache: `${CACHE_PREFIX}map-v1`,
    matches: (url) => url.pathname.startsWith("/map-assets/"),
    strategy: staleWhileRevalidate,
    // Glyph ranges are around 100 KB each, and Japanese labels need many of them.
    maxEntries: 150,
    maxAgeMs: 30 * DAY_MS,
  },
];
const KNOWN_CACHES = [SHELL_CACHE, ...RUNTIME_CACHES.map((rule) => rule.cache)];

/** Paths the worker never answers, not even for a navigation: they carry credentials or people's uploads. */
const UNTOUCHED = ["/api/", "/media/", "/tiles/"];

/** Response header added to stored copies: when they were stored (ms since epoch). */
const STORED_AT = "x-tokuly-stored-at";
/** Sweep a cache after this many additions (and once when the worker activates). */
const TRIM_EVERY = 30;

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;
  if (UNTOUCHED.some((prefix) => url.pathname.startsWith(prefix))) return;

  if (request.mode === "navigate") {
    event.respondWith(navigate(event));
    return;
  }
  // Signed requests and partial content are never stored.
  if (request.headers.has("authorization") || request.headers.has("range")) return;
  const rule = RUNTIME_CACHES.find((candidate) => candidate.matches(url));
  if (rule) event.respondWith(rule.strategy(event, rule));
});

/** A page load: always from the network, so nobody is served an old app; the offline page when there is none. */
async function navigate(event) {
  try {
    // With navigation preload the browser started the request while the worker was waking up.
    const response = (await event.preloadResponse) || (await fetch(event.request));
    // Catches up when the offline page could not be stored at install time.
    event.waitUntil(storeOfflinePage().catch(() => {}));
    return response;
  } catch {
    const cache = await caches.open(SHELL_CACHE);
    return (await cache.match(OFFLINE_URL)) || bareOfflinePage();
  }
}

/** Last resort when even the offline page is missing (it could never be fetched). */
function bareOfflinePage() {
  const html =
    '<!doctype html><html lang="ja"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">' +
    "<title>Tokuly</title><body style=\"font-family:system-ui,sans-serif;text-align:center;padding:4rem 1.5rem\">" +
    "<h1>オフラインみたい</h1><p>You are offline.</p><p><button onclick=\"location.reload()\">もう一度ためす / Try again</button></p>";
  return new Response(html, { status: 503, headers: { "content-type": "text/html; charset=utf-8" } });
}

let offlinePageStored = false;
async function storeOfflinePage() {
  if (offlinePageStored) return;
  const cache = await caches.open(SHELL_CACHE);
  if (!(await cache.match(OFFLINE_URL))) {
    // `reload`: the copy that is kept must come from the server, not from the browser's HTTP cache.
    const response = await fetch(new Request(OFFLINE_URL, { cache: "reload" }));
    if (!storable(response)) throw new Error(`offline page: ${response.status}`);
    await cache.put(OFFLINE_URL, response);
  }
  offlinePageStored = true;
}

async function enableNavigationPreload() {
  if (self.registration.navigationPreload) await self.registration.navigationPreload.enable();
}

/** Caches of earlier versions of this worker. */
async function dropForeignCaches() {
  const names = await caches.keys();
  await Promise.all(names.filter((name) => name.startsWith(CACHE_PREFIX) && !KNOWN_CACHES.includes(name)).map((name) => caches.delete(name)));
}

/** The stored copy if there is one, the network (storing the answer) otherwise. */
async function cacheFirst(event, rule) {
  const cache = await caches.open(rule.cache);
  const stored = await cache.match(event.request);
  if (stored && fresh(stored, rule)) return stored;
  try {
    return await fetchAndStore(event, cache, rule);
  } catch (error) {
    // Too old to keep, but better than nothing on a bad connection.
    if (stored) return stored;
    throw error;
  }
}

/** The stored copy at once, refreshed in the background for next time; the network when there is none. */
async function staleWhileRevalidate(event, rule) {
  const cache = await caches.open(rule.cache);
  const stored = await cache.match(event.request);
  const refreshed = fetchAndStore(event, cache, rule);
  if (stored && fresh(stored, rule)) {
    event.waitUntil(refreshed.catch(() => {}));
    return stored;
  }
  try {
    return await refreshed;
  } catch (error) {
    if (stored) return stored;
    throw error;
  }
}

async function fetchAndStore(event, cache, rule) {
  const response = await fetch(event.request);
  if (storable(response)) {
    event.waitUntil(
      stamped(response.clone())
        .then((copy) => cache.put(event.request, copy))
        .then(() => trimSometimes(rule))
        // A full disk must not fail the request that is being answered.
        .catch(() => {}),
    );
  }
  return response;
}

/** Whether a response may be kept: a complete, public answer from this origin. */
function storable(response) {
  if (response.status !== 200 || response.type !== "basic") return false;
  if (/no-store|private/i.test(response.headers.get("cache-control") || "")) return false;
  if (/\*|cookie|authorization/i.test(response.headers.get("vary") || "")) return false;
  // A server that answers a missing file with the app's page (200, HTML) must not poison the cache.
  const expectsPage = new URL(response.url).pathname.endsWith(".html");
  return expectsPage || !(response.headers.get("content-type") || "").includes("text/html");
}

/** A copy of `response` that remembers when it was stored. */
async function stamped(response) {
  const headers = new Headers(response.headers);
  headers.set(STORED_AT, String(Date.now()));
  return new Response(await response.blob(), { status: response.status, statusText: response.statusText, headers });
}

function fresh(response, rule) {
  const storedAt = Number(response.headers.get(STORED_AT));
  return Number.isFinite(storedAt) && Date.now() - storedAt < rule.maxAgeMs;
}

const additions = new Map();
async function trimSometimes(rule) {
  const count = (additions.get(rule.cache) || 0) + 1;
  additions.set(rule.cache, count % TRIM_EVERY);
  if (count >= TRIM_EVERY) await trim(rule);
}

/** Keeps a cache within its limits: drops what is too old, then the oldest entries beyond `maxEntries`. */
async function trim(rule) {
  const cache = await caches.open(rule.cache);
  const kept = [];
  // `keys()` lists entries in the order they were stored: oldest first.
  for (const request of await cache.keys()) {
    const response = await cache.match(request);
    if (response && fresh(response, rule)) kept.push(request);
    else await cache.delete(request);
  }
  for (const request of kept.slice(0, Math.max(0, kept.length - rule.maxEntries))) await cache.delete(request);
}

/** Forgets everything stored for offline use, then stores the offline page again. */
async function clearOfflineData() {
  const names = await caches.keys();
  await Promise.all(names.filter((name) => name.startsWith(CACHE_PREFIX)).map((name) => caches.delete(name)));
  additions.clear();
  offlinePageStored = false;
  await storeOfflinePage().catch(() => {});
}

// ── 3. Push ─────────────────────────────────────────────────────────

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
    badge: APP_BADGE,
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

// ── 4. Notification click ───────────────────────────────────────────

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

// ── 5. Subscription change ──────────────────────────────────────────

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

// ── 6. Messages from pages ──────────────────────────────────────────

self.addEventListener("message", (event) => {
  const type = event.data?.type;
  if (type === "tokuly:skip-waiting") {
    // The person tapped "update": activate now; every open page gets `controllerchange`.
    event.waitUntil(self.skipWaiting());
  } else if (type === "tokuly:clear-offline-data") {
    const reply = (ok) => event.ports[0]?.postMessage({ ok });
    event.waitUntil(clearOfflineData().then(() => reply(true), () => reply(false)));
  }
});
