import * as errors from "@tokuly/core/errors";
import i18n from "#/lib/i18n";

/** A message fit to show for any thrown value (`@tokuly/core/errors`), in the app's language. */
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
