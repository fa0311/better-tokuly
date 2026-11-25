import type { RefreshTokenStore } from "@tokuly/api";
import { readJson, readSecure, writeJson, writeSecure } from "#/platform/secure-storage";

/**
 * Where the refresh tokens of this device's accounts live: the active one
 * (the session's own store, rotated on every refresh) and the parked ones
 * (a main account, a 裏垢, a creator account), by their session ID. Only
 * this device ever holds them; the server never learns they belong together.
 */
const ACTIVE_KEY = "tokuly.session.active";
const PARKED_KEY = "tokuly.session.parked";

/** Every account this install holds, for forgetting them all at once. */
export const ACCOUNT_KEYS = [ACTIVE_KEY, PARKED_KEY] as const;

/** session ID → refresh token of each parked account. */
export type ParkedTokens = Record<string, string>;

export const activeTokenStore: RefreshTokenStore = {
  get: () => readSecure(ACTIVE_KEY),
  set: (token) => writeSecure(ACTIVE_KEY, token),
  clear: () => writeSecure(ACTIVE_KEY, null),
};

export function parkedTokens(): Promise<ParkedTokens> {
  return readJson<ParkedTokens>(PARKED_KEY, {});
}

/** Parks the active account: its token moves beside the others, under its session ID. */
export async function parkActive(sessionId: string): Promise<void> {
  const token = await activeTokenStore.get();
  if (!token || !sessionId) return;
  await writeJson(PARKED_KEY, { ...(await parkedTokens()), [sessionId]: token });
  await activeTokenStore.clear();
}

/**
 * Keeps the token a parked account's session was just rotated to: the one it
 * had is spent, and the server revokes a session whose spent token comes back.
 */
export async function rotateParked(sessionId: string, token: string): Promise<void> {
  await writeJson(PARKED_KEY, { ...(await parkedTokens()), [sessionId]: token });
}

/** Forgets a parked account (it became the active one, or it was signed out of this device). */
export async function forgetParked(sessionId: string): Promise<void> {
  const { [sessionId]: _gone, ...rest } = await parkedTokens();
  await writeJson(PARKED_KEY, rest);
}

/** Keeps only the parked accounts the server still knows (`ListDeviceAccounts` leaves the others out). */
export async function keepParked(sessionIds: ReadonlySet<string>): Promise<void> {
  const tokens = await parkedTokens();
  const kept = Object.fromEntries(Object.entries(tokens).filter(([id]) => sessionIds.has(id)));
  if (Object.keys(kept).length !== Object.keys(tokens).length) await writeJson(PARKED_KEY, kept);
}
