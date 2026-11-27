import * as errors from "@tokuly/core/errors";
import i18n from "#/lib/i18n";

export type { ErrorMessageOptions } from "@tokuly/core/errors";

/**
 * A message fit to show the user for any thrown value, in the language the
 * web app shows (`@tokuly/core/errors`): the shared `errors` namespace has
 * every reason, a feature's own wording is under `errors` in its namespace.
 */
export function errorMessage(error: unknown, options: errors.ErrorMessageOptions = {}): string {
  return errors.errorMessage(i18n, error, options);
}

/** The text for a reason that came without an error (`@tokuly/core/errors`); undefined if unknown. */
export function reasonText(
  reason: string,
  options: errors.ErrorMessageOptions = {},
): string | undefined {
  return errors.reasonText(i18n, reason, options);
}

/** A duration limit in words: "60秒", "10分", "3時間" ("60 sec", "10 min", "3 hr"). */
export function durationLimit(seconds: number): string {
  return errors.durationLimit(i18n, seconds);
}
