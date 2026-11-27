import type { identity } from "@tokuly/api";

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
 * Whether the birthdate screen shows now. `later` is the account that
 * answered 「あとで」 during this sign-in: the app forgets it when the
 * account signs out (or another one becomes active), so the next sign-in
 * asks again.
 */
export function asksForBirthdateNow(me: identity.Me | null, later: string | null): boolean {
  return me !== null && asksForBirthdate(me) && later !== me.user?.id;
}
