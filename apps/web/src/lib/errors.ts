import { Code, errorReason, isApiError } from "@tokuly/api";
import i18n from "#/lib/i18n";

/** The localized text for an API reason, from any feature's `errors` group or the shared list. */
function reasonMessage(reason: string): string | undefined {
  if (i18n.exists(reason, { ns: "errors" })) return i18n.t(reason, { ns: "errors" });
  // Features keep the reasons only they can hit under `errors.<REASON>` in their own namespace.
  for (const namespace of Object.keys(i18n.getDataByLanguage(i18n.language) ?? {})) {
    if (i18n.exists(`errors.${reason}`, { ns: namespace })) return i18n.t(`errors.${reason}`, { ns: namespace });
  }
  return undefined;
}

/**
 * A message fit to show the user for any thrown value: the localized text for
 * the API's error reason when there is one, a network hint when the request
 * never arrived, a generic apology otherwise.
 */
export function errorMessage(error: unknown): string {
  const reason = errorReason(error);
  const known = reason ? reasonMessage(reason) : undefined;
  if (known) return known;
  if (reason === "RATE_LIMITED" || (isApiError(error) && error.code === Code.ResourceExhausted)) return i18n.t("RATE_LIMITED", { ns: "errors" });
  if (!isApiError(error) || error.code === Code.Unavailable || error.code === Code.Unknown) return i18n.t("network", { ns: "errors" });
  return i18n.t("generic", { ns: "errors" });
}
