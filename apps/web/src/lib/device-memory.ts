/**
 * Small things the app remembers on this device about what a person has
 * already seen or put off (a suggestion dismissed, a level celebrated).
 * They are about where someone has been, so they follow three rules: one
 * entry per account (the key ends in the user ID), no coordinates or region
 * names in them (only short hashes: `opaqueHash` of `@tokuly/core/places`),
 * and nothing left behind on sign-out.
 */

export function readDeviceMemory<T>(key: string): T | null {
  try {
    return JSON.parse(window.localStorage.getItem(key) ?? "null") as T | null;
  } catch {
    return null;
  }
}

export function writeDeviceMemory(key: string, value: unknown): void {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Private mode or a full store: this is a convenience, not a promise.
  }
}

/** Removes every entry whose key starts with one of `prefixes`: the next person to sign in must not find them. */
export function forgetDeviceMemory(...prefixes: string[]): void {
  try {
    for (const key of Object.keys(window.localStorage))
      if (prefixes.some((prefix) => key.startsWith(prefix))) window.localStorage.removeItem(key);
  } catch {
    // Storage that cannot be read holds nothing to clear.
  }
}
