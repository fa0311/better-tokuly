/**
 * Signing out: what a feature keeps of the account in this browser outside
 * the query cache. It goes when the session ends, however it ends (signed
 * out here, from another device, by the server), so that the next person at
 * a shared computer finds nothing of it.
 *
 * A feature that keeps such things default-exports them from
 * `src/features/<area>/sign-out.ts`:
 *
 *   export default {
 *     storage: [DRAFTS_PREFIX],
 *     forget: () => closeCompose(),
 *   } satisfies SignOutCleanup;
 *
 * `stores/session.ts` runs every one when a signed-in session becomes
 * signed out, whatever is mounted. Not here: what the device keeps for itself
 * (the theme, the volume) and owed work kept by account until it reaches the
 * server (a stop of sharing, an unanswered gift).
 */
export interface SignOutCleanup {
  /** `localStorage` and `sessionStorage` entries: their keys, or how the keys start. */
  storage?: readonly string[];
  /** What the account left in memory: stores, timers, what plays. */
  forget?(): void;
}

const modules = import.meta.glob<{ default: SignOutCleanup }>("../features/*/sign-out.ts");

/** Every feature's cleanup (loaded when needed: nothing of it is in the first page's bundle). */
export async function signOutCleanups(): Promise<SignOutCleanup[]> {
  return Promise.all(Object.values(modules).map(async (load) => (await load()).default));
}

function removeEntries(storage: Storage, starts: readonly string[]): void {
  for (const key of Object.keys(storage))
    if (starts.some((start) => key.startsWith(start))) storage.removeItem(key);
}

/** Removes what the signed-out account left in this browser; a cleanup that fails does not stop the others. */
export async function forgetSignedOutAccount(cleanups?: readonly SignOutCleanup[]): Promise<void> {
  const all = cleanups ?? (await signOutCleanups());
  const starts = all.flatMap(({ storage }) => storage ?? []);
  for (const storage of [() => window.localStorage, () => window.sessionStorage]) {
    try {
      removeEntries(storage(), starts);
    } catch {
      // Storage that cannot be read holds nothing to remove.
    }
  }
  for (const { forget } of all) {
    try {
      forget?.();
    } catch (error) {
      console.error("sign-out: a cleanup failed", error);
    }
  }
}
