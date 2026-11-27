import { type identity, toMillis } from "@tokuly/api";

/**
 * The account was made before sign-up asked for a birthdate: the app asks for
 * it after signing in. Until it is given the account counts as under 18
 * (`Me.minor`). A bot has no birthdate of its own; it goes by its owner.
 */
export function asksForBirthdate(me: identity.Me): boolean {
  return !me.bot && me.birthdate === "";
}

/**
 * Under the minimum age by the birthdate given: the app shows only why, and
 * the way to delete the account, until that birthday. The birthdate cannot
 * be answered again.
 */
export function isTooYoung(me: identity.Me | null): boolean {
  return me?.tooYoung === true;
}

/**
 * A timer is not trusted with more than this at once: it does not run while
 * the device sleeps, and one of weeks does not fit in it at all.
 */
const LONGEST_WAIT_MS = 6 * 3600_000;
/** After the date, while the server still says too young (a clock ahead of the server's). */
const RETRY_MS = 60_000;

/** When the server lets a too-young account in (`Me.tooYoungUntil`, ms); 0 when it is not too young. */
export function tooYoungUntil(me: identity.Me | null): number {
  return isTooYoung(me) ? toMillis(me?.tooYoungUntil) : 0;
}

/**
 * Keeps a too-young account's app from waiting for a manual reload: at
 * `until` (`tooYoungUntil`) `recheck` asks again (a token refresh, which
 * brings the new `Me`). `onWake` subscribes to the app coming back to the
 * screen (a tab shown again, an app brought to the front): a device that
 * slept through the moment asks at once. Returns the function that stops it.
 */
export function watchTooYoung(
  until: number,
  recheck: () => Promise<unknown>,
  onWake: (listener: () => void) => () => void,
): () => void {
  if (until === 0) return () => {};
  let timer: ReturnType<typeof setTimeout> | undefined;
  let asking = false;
  let stopped = false;

  const schedule = (ms: number) => {
    clearTimeout(timer);
    timer = setTimeout(check, Math.min(Math.max(ms, 0), LONGEST_WAIT_MS));
  };
  async function check() {
    if (stopped || asking) return;
    const left = until - Date.now();
    if (left > 0) return schedule(left);
    asking = true;
    clearTimeout(timer);
    await recheck().catch(() => {});
    asking = false;
    if (!stopped) schedule(RETRY_MS);
  }

  const unsubscribe = onWake(() => {
    if (Date.now() >= until) void check();
  });
  schedule(until - Date.now());
  return () => {
    stopped = true;
    clearTimeout(timer);
    unsubscribe();
  };
}

/**
 * Whether the birthdate screen shows now. `later` is the account that
 * answered 「あとで」 during this sign-in: the app forgets it when the
 * account signs out (or another one becomes active), so the next sign-in
 * asks again.
 */
export function asksForBirthdateNow(me: identity.Me | null, later: string | null): boolean {
  return me !== null && asksForBirthdate(me) && later !== me.user?.id;
}
