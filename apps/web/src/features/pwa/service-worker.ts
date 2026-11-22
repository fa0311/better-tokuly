import { create } from "zustand";

/**
 * The page's side of the service worker (public/sw.js): registering it,
 * noticing that a newer version is waiting, and handing that version over
 * when the person asks for it. The worker never replaces a running version by
 * itself, so nobody's page changes underneath them.
 */

const SERVICE_WORKER_URL = "/sw.js";
/** The app is often left open for days: look for a new version this often when it is looked at again. */
const UPDATE_CHECK_EVERY_MS = 60 * 60_000;
/** Names of the worker's caches start with this (see public/sw.js). */
export const CACHE_PREFIX = "tokuly-";

export type WorkerStatus =
  /** No service workers in this browser. */
  | "unsupported"
  /** Being registered, or installing for the first time. */
  | "starting"
  /** Installed and running. */
  | "active"
  /** Could not be registered (blocked, offline on the first visit…). */
  | "failed";

interface WorkerState {
  status: WorkerStatus;
  /** A newer version is installed and waits for the go-ahead. */
  updateReady: boolean;
  /** Another tab accepted an update: the new worker runs, but this page is still the old app. */
  replaced: boolean;
  /** The server speaks a newer realtime protocol than this page was built for (see `protocol.ts`). */
  outdated: boolean;
}

export const useWorker = create<WorkerState>(() => ({
  status: "starting",
  updateReady: false,
  replaced: false,
  outdated: false,
}));

/** Whether this page runs an older app than the one it could be running: a reload is due. */
export function pageIsBehind(state: WorkerState): boolean {
  return state.updateReady || state.replaced || state.outdated;
}

let registration: ServiceWorkerRegistration | null = null;
let started = false;
/** The person asked for the update on this page: reload when the new worker takes over. */
let reloadOnTakeover = false;
let lastUpdateCheck = 0;

function refresh(): void {
  if (!registration) return;
  useWorker.setState({
    status: registration.active ? "active" : "starting",
    // A worker that waits while another one runs is an update; the very first one never waits.
    updateReady: Boolean(registration.waiting && registration.active),
  });
}

function follow(worker: ServiceWorker | null): void {
  worker?.addEventListener("statechange", refresh);
}

/** Registers the worker (once) and keeps `useWorker` current from then on. */
export function startServiceWorker(): void {
  if (started || typeof navigator === "undefined") return;
  started = true;
  if (!("serviceWorker" in navigator)) {
    useWorker.setState({ status: "unsupported" });
    return;
  }

  // A page that already had a worker and gets another one was updated from elsewhere.
  let controlled = Boolean(navigator.serviceWorker.controller);
  navigator.serviceWorker.addEventListener("controllerchange", () => {
    if (reloadOnTakeover) {
      window.location.reload();
      return;
    }
    // The first worker taking this page changes nothing for it. An update accepted in
    // another tab does: this page still runs the old app, so it offers a reload too.
    if (controlled) useWorker.setState({ replaced: true });
    controlled = true;
    refresh();
  });
  document.addEventListener("visibilitychange", () => {
    if (
      document.visibilityState === "visible" &&
      Date.now() - lastUpdateCheck > UPDATE_CHECK_EVERY_MS
    )
      void checkForUpdate();
  });

  navigator.serviceWorker
    .register(SERVICE_WORKER_URL)
    .then((registered) => {
      registration = registered;
      lastUpdateCheck = Date.now();
      registered.addEventListener("updatefound", () => {
        follow(registered.installing);
        refresh();
      });
      follow(registered.installing);
      follow(registered.waiting);
      refresh();
    })
    .catch(() => useWorker.setState({ status: "failed" }));
}

/** Asks the server whether there is a newer worker. Resolves to whether one is ready to take over. */
export async function checkForUpdate(): Promise<boolean> {
  if (!registration) return false;
  lastUpdateCheck = Date.now();
  try {
    await registration.update();
  } catch {
    // Offline, or the server is away: whatever is installed stays.
    return useWorker.getState().updateReady;
  }
  // `update()` resolves when the new script was fetched; installing it takes a moment more.
  const installing = registration.installing;
  if (installing) {
    await new Promise<void>((resolve) => {
      const settled = () => {
        if (installing.state === "installing") return;
        installing.removeEventListener("statechange", settled);
        resolve();
      };
      installing.addEventListener("statechange", settled);
      settled();
    });
  }
  refresh();
  return useWorker.getState().updateReady;
}

/** Switches to the waiting version and reloads the page into it. */
export function applyUpdate(): void {
  const waiting = registration?.waiting;
  if (!waiting) {
    // Another tab already switched: a reload is all this page needs.
    window.location.reload();
    return;
  }
  reloadOnTakeover = true;
  waiting.postMessage({ type: "tokuly:skip-waiting" });
  // Should the takeover never be announced, the reload still happens.
  window.setTimeout(() => window.location.reload(), 4000);
}

/** Posts `message` to the worker and resolves to its `{ ok }` answer (false when it does not answer). */
function ask(
  worker: ServiceWorker,
  message: { type: string },
  timeoutMs = 10_000,
): Promise<boolean> {
  return new Promise((resolve) => {
    const channel = new MessageChannel();
    const timer = window.setTimeout(() => resolve(false), timeoutMs);
    channel.port1.onmessage = (event: MessageEvent<{ ok?: boolean }>) => {
      window.clearTimeout(timer);
      resolve(event.data?.ok === true);
    };
    worker.postMessage(message, [channel.port2]);
  });
}

/**
 * Forgets everything the worker stored for offline use. The worker does it
 * (and stores the offline page again); without one, its caches are removed
 * from here.
 */
export async function clearOfflineData(): Promise<void> {
  const worker = registration?.active;
  if (worker && (await ask(worker, { type: "tokuly:clear-offline-data" }))) return;
  if (typeof caches === "undefined") return;
  const names = await caches.keys();
  await Promise.all(
    names.filter((name) => name.startsWith(CACHE_PREFIX)).map((name) => caches.delete(name)),
  );
}
