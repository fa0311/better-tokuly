import type { Api } from "@tokuly/api";
import { useSessionStore } from "#/features/session/session";
import { api } from "#/lib/api";

/**
 * The account a piece of work was started as: its session and the API object
 * that speaks for it. Captured before the first await, and checked after every
 * one: a switch or a sign-out replaces the active API (and the token store
 * behind it), so anything that went on with the old object afterwards could
 * end up speaking as the next account.
 */
export interface Account {
  sessionId: string;
  userId: string;
  api: Api;
}

/** The work's account is no longer the active one: it stops, and waits for it to come back. */
export class AccountChanged extends Error {
  constructor() {
    super("the account changed");
    this.name = "AccountChanged";
  }
}

/** The active account, or `null` while nobody is signed in. */
export function currentAccount(): Account | null {
  const { state } = useSessionStore.getState();
  if (state.status !== "signedIn") return null;
  return { sessionId: state.sessionId, userId: state.me.user?.id ?? "", api: api() };
}

export function isActive(account: Account): boolean {
  const { state } = useSessionStore.getState();
  return (
    state.status === "signedIn" && state.sessionId === account.sessionId && api() === account.api
  );
}

/** Throws `AccountChanged` unless `account` is still the one in use. */
export function ensureActive(account: Account): void {
  if (!isActive(account)) throw new AccountChanged();
}
