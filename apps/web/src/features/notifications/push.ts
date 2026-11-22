import { common, type notify } from "@tokuly/api";
import i18n from "#/lib/i18n";
import { pushClient } from "./api";

/**
 * Web Push plumbing: the service worker, the browser's push subscription and
 * its registration with the server. No React in here; `use-push.ts` turns it
 * into state for the screens.
 */

const SERVICE_WORKER_URL = "/sw.js";
const SERVICE_WORKER_TIMEOUT_MS = 8000;

/** Who turned push on in this browser, and what was last registered for them. */
const REGISTRATION_KEY = "tokuly.push.registration";
/** When the "turn on push" prompt in the inbox was last dismissed. */
const PROMPT_DISMISSED_KEY = "tokuly.push.promptDismissedAt";

/** A registration is refreshed on app start when it is older than this. */
const REFRESH_AFTER_MS = 24 * 60 * 60_000;
/** A dismissed prompt stays away this long. */
const PROMPT_SNOOZE_MS = 30 * 24 * 60 * 60_000;

/** Why push could not be turned on. */
export type PushFailure =
  /** No service worker / Push API in this browser. */
  | "unsupported"
  /** Notifications are blocked for this site. */
  | "denied"
  /** The permission prompt was closed without an answer. */
  | "dismissed"
  /** The server offers no Web Push. */
  | "unavailable"
  /** The browser's push service refused the subscription. */
  | "subscribe";

export class PushError extends Error {
  constructor(
    readonly failure: PushFailure,
    cause?: unknown,
  ) {
    super(`push: ${failure}`, { cause });
  }
}

/** Whether this browser can receive Web Push at all. */
export function pushSupported(): boolean {
  return (
    typeof window !== "undefined" &&
    "serviceWorker" in navigator &&
    "PushManager" in window &&
    "Notification" in window
  );
}

/** iPhones and iPads only offer push to apps added to the home screen. */
export function needsHomeScreenInstall(): boolean {
  const apple =
    /iPhone|iPad|iPod/.test(navigator.userAgent) ||
    (navigator.userAgent.includes("Macintosh") && navigator.maxTouchPoints > 1);
  return apple && !window.matchMedia("(display-mode: standalone)").matches;
}

let worker: Promise<ServiceWorkerRegistration | null> | null = null;

/** The app's service worker, registered on first use and ready (active). `null` when unavailable. */
export function serviceWorker(): Promise<ServiceWorkerRegistration | null> {
  if (typeof navigator === "undefined" || !("serviceWorker" in navigator))
    return Promise.resolve(null);
  worker ??= Promise.race([
    navigator.serviceWorker.register(SERVICE_WORKER_URL).then(() => navigator.serviceWorker.ready),
    // `ready` never settles when the worker cannot start; nothing may wait on it forever.
    new Promise<null>((resolve) => setTimeout(resolve, SERVICE_WORKER_TIMEOUT_MS, null)),
  ])
    .catch(() => null)
    .then((registration) => {
      if (!registration) worker = null; // Try again next time (offline, a blocked script...).
      return registration;
    });
  return worker;
}

/**
 * This browser's push subscription, if it has one. Throws while the service
 * worker cannot be reached: "not known yet" must not pass for "none".
 */
export async function currentSubscription(): Promise<PushSubscription | null> {
  const registration = await serviceWorker();
  if (!registration) throw new PushError("unsupported");
  return registration.pushManager.getSubscription();
}

function base64UrlToBytes(value: string): Uint8Array<ArrayBuffer> {
  const base64 = value.replace(/-/g, "+").replace(/_/g, "/");
  const binary = atob(base64 + "=".repeat((4 - (base64.length % 4)) % 4));
  const bytes = new Uint8Array(new ArrayBuffer(binary.length));
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

function sameBytes(a: ArrayBuffer | null | undefined, b: Uint8Array): boolean {
  if (!a) return false;
  const left = new Uint8Array(a);
  return left.length === b.length && left.every((byte, index) => byte === b[index]);
}

/** This browser's subscription for the server's VAPID key, creating it when needed. */
async function subscribe(vapidPublicKey: string): Promise<PushSubscription> {
  const registration = await serviceWorker();
  if (!registration) throw new PushError("unsupported");
  if (!vapidPublicKey) throw new PushError("unavailable");
  const key = base64UrlToBytes(vapidPublicKey);
  try {
    const existing = await registration.pushManager.getSubscription();
    if (existing) {
      if (sameBytes(existing.options.applicationServerKey, key)) return existing;
      // Made for another server key: nothing this server sends would arrive.
      await existing.unsubscribe();
    }
    return await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: key,
    });
  } catch (error) {
    throw new PushError("subscribe", error);
  }
}

/** "Chrome (macOS)": how this browser is named in the device list. */
export function deviceName(userAgent: string = navigator.userAgent): string {
  const browser = /Edg\//.test(userAgent)
    ? "Edge"
    : /OPR\//.test(userAgent)
      ? "Opera"
      : /Firefox\//.test(userAgent)
        ? "Firefox"
        : /Chrome\//.test(userAgent)
          ? "Chrome"
          : /Safari\//.test(userAgent)
            ? "Safari"
            : "Browser";
  const os = /Android/.test(userAgent)
    ? "Android"
    : /iPhone|iPad|iPod/.test(userAgent)
      ? "iOS"
      : /Mac OS X/.test(userAgent)
        ? "macOS"
        : /Windows/.test(userAgent)
          ? "Windows"
          : /CrOS/.test(userAgent)
            ? "ChromeOS"
            : /Linux/.test(userAgent)
              ? "Linux"
              : "";
  return os ? `${browser} (${os})` : browser;
}

/** The signed-in session a registration is made for. */
export interface PushOwner {
  userId: string;
  sessionId: string;
}

interface StoredRegistration extends PushOwner {
  endpoint: string;
  at: number;
}

function storedRegistration(): StoredRegistration | null {
  try {
    const value = JSON.parse(
      window.localStorage.getItem(REGISTRATION_KEY) ?? "null",
    ) as StoredRegistration | null;
    return value && typeof value.userId === "string" && typeof value.endpoint === "string"
      ? value
      : null;
  } catch {
    return null;
  }
}

/** Hands the subscription to the server for the calling session and remembers who it belongs to. */
async function register(
  subscription: PushSubscription,
  owner: PushOwner,
): Promise<notify.PushDevice | undefined> {
  const keys = subscription.toJSON().keys ?? {};
  const response = await pushClient().registerDevice({
    token: {
      case: "webPush",
      value: { endpoint: subscription.endpoint, p256dh: keys.p256dh ?? "", auth: keys.auth ?? "" },
    },
    platform: common.Platform.WEB,
    locale: i18n.language,
    deviceName: deviceName(),
  });
  const stored: StoredRegistration = { ...owner, endpoint: subscription.endpoint, at: Date.now() };
  window.localStorage.setItem(REGISTRATION_KEY, JSON.stringify(stored));
  return response.device;
}

/**
 * Turns push on for this browser: asks for permission when the browser has
 * not been asked yet (so only ever call this from a tap), subscribes and
 * registers. Throws `PushError`, or the API error of the registration.
 */
export async function enablePush(owner: PushOwner): Promise<notify.PushDevice | undefined> {
  if (!pushSupported()) throw new PushError("unsupported");
  const permission =
    Notification.permission === "default"
      ? await Notification.requestPermission()
      : Notification.permission;
  if (permission !== "granted")
    throw new PushError(permission === "denied" ? "denied" : "dismissed");
  const config = await pushClient().getPushConfig({});
  return register(await subscribe(config.vapidPublicKey), owner);
}

/** Turns push off for this browser: the server forgets the session's registration, the browser its subscription. */
export async function disablePush(): Promise<void> {
  window.localStorage.removeItem(REGISTRATION_KEY);
  await pushClient().unregisterDevice({ deviceId: "" });
  try {
    await (await currentSubscription())?.unsubscribe();
  } catch {
    // The server no longer sends here; an orphaned subscription is replaced the next time push is turned on.
  }
}

/**
 * Keeps an existing opt-in working, without ever prompting: run when the app
 * starts. A registration belongs to a session, and subscriptions expire, so
 * for the person who turned push on in this browser it is renewed when the
 * session or the subscription changed, or once a day.
 */
export async function refreshPushRegistration(owner: PushOwner): Promise<void> {
  if (!pushSupported() || Notification.permission !== "granted") return;
  const stored = storedRegistration();
  if (!stored || stored.userId !== owner.userId) return;
  try {
    const existing = await currentSubscription();
    const fresh =
      existing?.endpoint === stored.endpoint &&
      stored.sessionId === owner.sessionId &&
      Date.now() - stored.at < REFRESH_AFTER_MS;
    if (fresh) return;
    const subscription =
      existing ?? (await subscribe((await pushClient().getPushConfig({})).vapidPublicKey));
    await register(subscription, owner);
  } catch {
    // Not worth interrupting anyone: the settings page shows the real state, the next start retries.
  }
}

/** Closes system notifications that were read or removed in the app (all of them without `ids`). */
export async function closeShownNotifications(ids?: string[]): Promise<void> {
  if (!pushSupported() || Notification.permission !== "granted") return;
  try {
    const registration = await navigator.serviceWorker.getRegistration();
    for (const shown of (await registration?.getNotifications()) ?? []) {
      const id = (shown.data as { id?: string } | null)?.id;
      if (!ids || (id && ids.includes(id))) shown.close();
    }
  } catch {
    // Purely cosmetic.
  }
}

/** The unread count on the installed app's icon (Badging API), where there is one. */
export function setAppBadge(count: number): void {
  const badging = navigator as Navigator & {
    setAppBadge?(count: number): Promise<void>;
    clearAppBadge?(): Promise<void>;
  };
  void (count > 0 ? badging.setAppBadge?.(count) : badging.clearAppBadge?.())?.catch(() => {});
}

/** Whether the inbox may suggest turning push on (not for a while after it was waved away). */
export function promptSnoozed(): boolean {
  const at = Number(window.localStorage.getItem(PROMPT_DISMISSED_KEY) ?? 0);
  return Number.isFinite(at) && Date.now() - at < PROMPT_SNOOZE_MS;
}

export function snoozePrompt(): void {
  window.localStorage.setItem(PROMPT_DISMISSED_KEY, String(Date.now()));
}
