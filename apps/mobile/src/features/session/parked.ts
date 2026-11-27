import type { Api } from "@tokuly/api";
import { apiWith } from "#/lib/api";
import { parkedTokenStore, parkedTokens } from "./vault";

/**
 * Everything that reads or rotates a parked account's refresh token (a
 * switch, a removal, parking an account, work done as a parked session) runs
 * one after the other: a token sent twice — a switch and a refresh of the
 * same session at once — revokes that session (REFRESH_TOKEN_REUSE).
 */
let parkedQueue: Promise<unknown> = Promise.resolve();

export function withParkedTokens<T>(work: () => Promise<T>): Promise<T> {
  const run = parkedQueue.then(work);
  parkedQueue = run.catch(() => {});
  return run;
}

/**
 * Runs `work` as a parked account's session without making it the active
 * one (what it still owes the server, such as removing a push registration
 * a switch could not remove). Resolves to `null` when the device no longer
 * holds that account; throws when its session cannot be resumed now.
 */
export function asParked<T>(
  sessionId: string,
  work: (account: Api) => Promise<T>,
): Promise<T | null> {
  return withParkedTokens(async () => {
    if (!(await parkedTokens())[sessionId]) return null;
    const account = apiWith(parkedTokenStore(sessionId));
    // A refresh, not a restore: that would wait out a rate limit, holding up the device's tokens.
    if (!(await account.session.refresh()))
      throw new Error("the parked session could not be resumed");
    return work(account);
  });
}
