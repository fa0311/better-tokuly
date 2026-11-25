import * as errors from "@tokuly/core/errors";
import i18n from "#/lib/i18n";

export type { ErrorMessageOptions } from "@tokuly/core/errors";

/**
 * A message fit to show the user for any thrown value, in the language the
 * web app shows (`@tokuly/core/errors`): reasons any screen can hit live in
 * the shared `errors` namespace, reasons of one feature under `errors` in
 * that feature's namespace.
 */
export function errorMessage(error: unknown, options: errors.ErrorMessageOptions = {}): string {
  return errors.errorMessage(i18n, error, options);
}

/** A duration limit in words: "60秒", "10分", "3時間" ("60 sec", "10 min", "3 hr"). */
export function durationLimit(seconds: number): string {
  return errors.durationLimit(i18n, seconds);
}
