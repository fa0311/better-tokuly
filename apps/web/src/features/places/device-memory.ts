/**
 * Small things the app remembers on this device about what a person has
 * already seen or put off (a suggestion dismissed, a level celebrated).
 * They are about where someone has been, so they follow three rules: one
 * entry per account (the key ends in the user ID), no coordinates or region
 * names in them (only short hashes), and nothing left behind on sign-out.
 */

/**
 * A short, one-way fingerprint of a text (FNV-1a, 32 bits, base 36). Short
 * on purpose: many different inputs share each value, so a stored hash
 * cannot be turned back into the place it came from, while two things the
 * same account remembers practically never collide.
 */
export function opaqueHash(text: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < text.length; index++) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(36);
}

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
    for (const key of Object.keys(window.localStorage)) if (prefixes.some((prefix) => key.startsWith(prefix))) window.localStorage.removeItem(key);
  } catch {
    // Storage that cannot be read holds nothing to clear.
  }
}
