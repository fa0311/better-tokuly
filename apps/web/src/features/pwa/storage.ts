import { CACHE_PREFIX } from "./service-worker";

/**
 * What the app keeps on the device: how much room it takes, and what the
 * service worker stored for a bad connection.
 */

export interface StorageUse {
  usedBytes: number;
  /** How much the browser lets this site use at most. */
  quotaBytes: number;
}

/** The room Tokuly takes on this device, or `null` where the browser does not say. */
export async function storageUse(): Promise<StorageUse | null> {
  if (typeof navigator === "undefined" || !navigator.storage?.estimate) return null;
  try {
    const { usage, quota } = await navigator.storage.estimate();
    if (typeof usage !== "number" || typeof quota !== "number" || quota <= 0) return null;
    return { usedBytes: usage, quotaBytes: quota };
  } catch {
    return null;
  }
}

export interface OfflineData {
  /** The offline page is stored: a page load without a connection ends there instead of in the browser's error. */
  ready: boolean;
  /** Files stored for next time (scripts, styles, fonts, map fonts). */
  files: number;
}

const OFFLINE_PAGE = "/offline.html";

/** What the service worker has stored. */
export async function offlineData(): Promise<OfflineData> {
  if (typeof caches === "undefined") return { ready: false, files: 0 };
  try {
    const names = (await caches.keys()).filter((name) => name.startsWith(CACHE_PREFIX));
    let files = 0;
    let ready = false;
    for (const name of names) {
      const cache = await caches.open(name);
      const requests = await cache.keys();
      files += requests.length;
      ready ||= requests.some((request) => new URL(request.url).pathname === OFFLINE_PAGE);
    }
    return { ready, files };
  } catch {
    // Storage can be closed to the page (some private modes).
    return { ready: false, files: 0 };
  }
}

const UNITS = ["kilobyte", "megabyte", "gigabyte"] as const;

/** "12.3 MB": a size as people read it, in the language's number format. Nothing is smaller than a kilobyte. */
export function formatBytes(bytes: number, language: string): string {
  let value = Math.max(0, bytes) / 1000;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit++;
  }
  // Whole kilobytes (a few bytes still show as 1 kB); one decimal above, but "120 MB", not "120.0 MB".
  if (unit === 0) value = Math.ceil(value);
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return new Intl.NumberFormat(language, { style: "unit", unit: UNITS[unit], unitDisplay: "short", maximumFractionDigits: digits }).format(value);
}

/** Share of the quota in use, 0–100, never shown as nothing when something is stored. */
export function usedPercent(use: StorageUse): number {
  const percent = (use.usedBytes / use.quotaBytes) * 100;
  return use.usedBytes > 0 ? Math.min(100, Math.max(1, percent)) : 0;
}
