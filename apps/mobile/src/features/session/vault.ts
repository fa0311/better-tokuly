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
const DEPARTED_KEY = "tokuly.session.departed";

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

/**
 * Parks the active account: its token moves beside the others, under its
 * session ID. All or nothing: an account that stays the active one goes on
 * rotating its token, and a copy among the parked ones would be a spent one.
 */
export async function parkActive(sessionId: string): Promise<void> {
  const token = await activeTokenStore.get();
  if (!token || !sessionId) return;
  const parked = await parkedTokens();
  await writeJson(PARKED_KEY, { ...parked, [sessionId]: token });
  try {
    await activeTokenStore.clear();
  } catch (error) {
    await writeJson(PARKED_KEY, parked);
    throw error;
  }
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

/**
 * The token store of one parked account: a refresh made for it (work its
 * session still owes, `asParked`) keeps the rotated token in its place, and a
 * session the server ended is forgotten.
 */
export function parkedTokenStore(sessionId: string): RefreshTokenStore {
  return {
    get: async () => (await parkedTokens())[sessionId] ?? null,
    set: (token) => rotateParked(sessionId, token),
    clear: () => forgetParked(sessionId),
  };
}

/** A session that left this device: its refresh token, and whether all of its account's sessions were to end. */
export interface Departed {
  token: string;
  allSessions: boolean;
}

/**
 * Accounts signed out of this device whose server may not know yet (signed
 * out offline, or left behind by a previous install), by session ID: kept
 * only until the server has removed their push registrations and ended them
 * — every session of the account when that was asked (`settleDeparted`).
 * The app shows nobody signed in for them.
 */
export function departedTokens(): Promise<Record<string, Departed>> {
  return readJson<Record<string, Departed>>(DEPARTED_KEY, {});
}

export async function addDeparted(sessionId: string, departed: Departed): Promise<void> {
  await writeJson(DEPARTED_KEY, { ...(await departedTokens()), [sessionId]: departed });
}

export async function forgetDeparted(sessionId: string): Promise<void> {
  const { [sessionId]: _gone, ...rest } = await departedTokens();
  await writeJson(DEPARTED_KEY, rest);
}

/**
 * The active store may still hold the token of an account that signed out
 * (it was set aside, then the phone refused to empty the store): that
 * account left, and is not resumed at the next start.
 */
export async function dropDepartedActive(): Promise<void> {
  const active = await activeTokenStore.get();
  if (!active) return;
  const departed = Object.values(await departedTokens());
  if (departed.some(({ token }) => token === active)) await activeTokenStore.clear();
}

/** The token store of one departed session: a refresh keeps the rotated token, a refused one ends it here. */
export function departedTokenStore(sessionId: string): RefreshTokenStore {
  return {
    get: async () => (await departedTokens())[sessionId]?.token ?? null,
    set: async (token) => {
      const held = (await departedTokens())[sessionId];
      await addDeparted(sessionId, { token, allSessions: held?.allSessions ?? false });
    },
    clear: () => forgetDeparted(sessionId),
  };
}

/**
 * A new install found the accounts of a previous one in the keychain (iOS
 * keeps it across an uninstall): they are not signed in here, but their
 * sessions may be alive on the server with this phone's push token, so they
 * go aside as departed, to be ended there.
 */
export async function departPreviousInstall(): Promise<void> {
  const active = await activeTokenStore.get();
  const parked = await parkedTokens();
  if (active) await addDeparted("previous-install", { token: active, allSessions: false });
  for (const [sessionId, token] of Object.entries(parked))
    await addDeparted(sessionId, { token, allSessions: false });
}
