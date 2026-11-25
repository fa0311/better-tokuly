import * as SecureStore from "expo-secure-store";

/**
 * Small values in the platform's secure storage (Keychain, Keystore). Keys
 * may only hold letters, digits, `.`, `-` and `_`. Readable after the first
 * unlock, so the background location task can sign in while the phone is
 * locked in a pocket.
 */
const OPTIONS: SecureStore.SecureStoreOptions = {
  keychainAccessible: SecureStore.AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY,
};

export function readSecure(key: string): Promise<string | null> {
  return SecureStore.getItemAsync(key, OPTIONS);
}

export async function writeSecure(key: string, value: string | null): Promise<void> {
  if (value === null) await SecureStore.deleteItemAsync(key, OPTIONS);
  else await SecureStore.setItemAsync(key, value, OPTIONS);
}

export async function readJson<T>(key: string, fallback: T): Promise<T> {
  const raw = await readSecure(key);
  if (!raw) return fallback;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

export function writeJson(key: string, value: unknown): Promise<void> {
  return writeSecure(key, JSON.stringify(value));
}
