import { Code, errorReason, isApiError } from "@tokuly/api";
import i18n from "#/lib/i18n";

export interface ErrorMessageOptions {
  /**
   * The i18n namespace of the feature that is showing the error. Its own
   * `errors.<REASON>` text is preferred, so a reason several features can hit
   * (an upload that failed, a bad cursor) reads right on each of their screens.
   */
  namespace?: string;
}

/**
 * The localized text for an API reason: the calling feature's own wording
 * first, then the shared list (`i18n/<lang>/errors.ts`: reasons any screen
 * can hit, such as a failed upload, in neutral words), then any other
 * feature's `errors` group.
 */
function reasonMessage(reason: string, namespace?: string): string | undefined {
  if (namespace && i18n.exists(`errors.${reason}`, { ns: namespace }))
    return i18n.t(`errors.${reason}`, { ns: namespace });
  if (i18n.exists(reason, { ns: "errors" })) return i18n.t(reason, { ns: "errors" });
  // Features keep the reasons only they can hit under `errors.<REASON>` in their own namespace.
  for (const other of Object.keys(i18n.getDataByLanguage(i18n.language) ?? {})) {
    if (i18n.exists(`errors.${reason}`, { ns: other }))
      return i18n.t(`errors.${reason}`, { ns: other });
  }
  return undefined;
}

/**
 * A message fit to show the user for any thrown value: the localized text for
 * the API's error reason when there is one, a network hint when the request
 * never arrived, a generic apology otherwise.
 */
export function errorMessage(error: unknown, options: ErrorMessageOptions = {}): string {
  const reason = errorReason(error);
  const known = reason ? reasonMessage(reason, options.namespace) : undefined;
  if (known) return known;
  if (reason === "RATE_LIMITED" || (isApiError(error) && error.code === Code.ResourceExhausted))
    return i18n.t("RATE_LIMITED", { ns: "errors" });
  if (!isApiError(error) || error.code === Code.Unavailable || error.code === Code.Unknown)
    return i18n.t("network", { ns: "errors" });
  return i18n.t("generic", { ns: "errors" });
}
