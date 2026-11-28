/**
 * Signing out: nothing of the account stays in this browser outside the
 * query cache. It goes when the session ends, however it ends (signed out
 * here, from another device, by the server), so that the next person at a
 * shared computer finds nothing of it.
 *
 * Storage needs no feature's code: every `tokuly` entry goes except those
 * this browser keeps for itself, listed below. What the account left in
 * memory (stores, timers, what plays) a feature forgets in
 * `src/features/<area>/sign-out.ts`:
 *
 *   export default { forget: () => closeCompose() } satisfies SignOutCleanup;
 *
 * Those files load when needed (nothing of them is in the first page's
 * bundle). One that does not arrive, after a few tries, is made up for by
 * loading the page anew: what was in memory goes with it.
 *
 * `stores/session.ts` runs this when a signed-in session becomes signed out,
 * whatever is mounted; signing out here returns once it ran, before the next
 * screen.
 */
export interface SignOutCleanup {
  /** What the account left in memory: stores, timers, what plays. */
  forget(): void;
}

/**
 * The storage entries this browser keeps after a sign-out (exact keys, or how
 * the keys start when they end in `.`), and why. Every other entry starting
 * with `tokuly` is the account's.
 */
export const KEPT_STORAGE: Readonly<Record<string, string>> = {
  "tokuly.deviceId": "this browser, not an account",
  "tokuly.theme": "the look of this browser",
  "tokuly.dopa": "the look of this browser",
  "tokuly.signedIn": "the session's own hint, removed by stores/session.ts",
  "tokuly.accountChanged": "tabs telling each other of a switch",
  "tokuly.push.registration": "the push subscription, taken back before the sign-out",
  "tokuly.push.promptDismissedAt": "this browser's answer to the prompt",
  "tokuly.pwa.installDismissedAt": "this browser's answer to the prompt",
  "tokuly.map.sessionEnding": "owed: a stop of sharing not yet sent",
  "tokuly.map.sharing.": "owed: a stop of sharing not yet sent, by account",
  "tokuly.map.stopOwed.": "owed: a stop of sharing not yet sent, by account",
  "tokuly.map.sequence.": "owed: the order of the account's reports",
  "tokuly.live.gift.": "owed: an unanswered gift, by account",
  "tokuly.party.guestName": "a guest's seat at a watch party, without an account",
  "tokuly.party.guest.": "a guest's seat at a watch party, without an account",
  "tokuly.spaces.microphone": "this device's microphone",
  "tokuly.spaces.speaker": "this device's speaker",
  "tokuly.shorts.muted": "this device's sound",
  "tokuly.player.theatre": "this device's player",
  "tokuly.player.volume": "this device's player",
  "tokuly.player.muted": "this device's player",
  "tokuly.player.captions": "this device's player",
};

const kept = (key: string) =>
  Object.keys(KEPT_STORAGE).some((entry) =>
    entry.endsWith(".") ? key.startsWith(entry) : key === entry,
  );

function removeAccountEntries(): void {
  for (const storage of [() => window.localStorage, () => window.sessionStorage]) {
    try {
      const store = storage();
      for (const key of Object.keys(store))
        if (/^tokuly[._]/.test(key) && !kept(key)) store.removeItem(key);
    } catch {
      // Storage that cannot be read holds nothing to remove.
    }
  }
}

type Load = () => Promise<{ default: SignOutCleanup }>;

const modules = import.meta.glob<{ default: SignOutCleanup }>("../features/*/sign-out.ts");

/** How long to wait before each new try of the cleanups that did not arrive. */
const RETRY_DELAYS_MS = [500, 1500];

/** Runs every feature's cleanup that arrives; returns whether every one did. */
async function forgetInMemory(loads: readonly Load[]): Promise<boolean> {
  let missing = loads;
  for (const delay of [0, ...RETRY_DELAYS_MS]) {
    if (delay) await new Promise((resolve) => setTimeout(resolve, delay));
    const results = await Promise.allSettled(missing.map((load) => load()));
    missing = missing.filter((_, index) => results[index]?.status === "rejected");
    for (const result of results) {
      if (result.status === "rejected") continue;
      try {
        result.value.default.forget();
      } catch (error) {
        console.error("sign-out: a cleanup failed", error);
      }
    }
    if (missing.length === 0) return true;
  }
  return false;
}

/**
 * Removes what the signed-out account left in this browser. When a feature's
 * cleanup cannot be loaded, the page loads anew instead and this never returns.
 */
export async function forgetSignedOutAccount(
  loads: readonly Load[] = Object.values(modules),
): Promise<void> {
  removeAccountEntries();
  if (await forgetInMemory(loads)) return;
  console.error("sign-out: a cleanup could not be loaded; starting the page anew");
  window.location.replace("/");
  await new Promise<never>(() => {});
}
