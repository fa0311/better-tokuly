import type { identity } from "@tokuly/api";

/**
 * The account was made before sign-up asked for a birthdate: the app asks for
 * it after signing in. Until it is given the account counts as under 18
 * (`Me.minor`). A bot has no birthdate of its own; it goes by its owner.
 */
export function asksForBirthdate(me: identity.Me): boolean {
  return !me.bot && me.birthdate === "";
}
