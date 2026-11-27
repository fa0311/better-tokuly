import { Code, errorReason, isApiError } from "@tokuly/api";
import type { i18n as I18n } from "i18next";
import { AssetFailedError } from "./asset-error";

export interface ErrorMessageOptions {
  /**
   * The i18n namespace of the feature that is showing the error. Its own
   * `errors.<REASON>` text is preferred, so a reason several features can hit
   * (an upload that failed, a name too long) reads right on each of their screens.
   */
  namespace?: string;
}

/**
 * The localized text for an API reason, also one that came without an error
 * (a draft that failed, a refused name): the calling feature's own wording
 * first, then the shared one (the `errors` namespace, which has every reason
 * the API returns, so the text never depends on which features' strings an
 * app has loaded). Undefined for a reason nobody knows.
 */
export function reasonText(
  i18n: I18n,
  reason: string,
  { namespace }: ErrorMessageOptions = {},
): string | undefined {
  if (namespace && i18n.exists(`errors.${reason}`, { ns: namespace }))
    return i18n.t(`errors.${reason}`, { ns: namespace });
  if (i18n.exists(reason, { ns: "errors" })) return i18n.t(reason, { ns: "errors" });
  return undefined;
}

/**
 * A message fit to show the user for any thrown value: the localized text for
 * the API's error reason when there is one, a network hint when the request
 * never arrived, a generic apology otherwise.
 */
export function errorMessage(
  i18n: I18n,
  error: unknown,
  options: ErrorMessageOptions = {},
): string {
  if (error instanceof AssetFailedError) return assetFailureMessage(i18n, error, options);
  const reason = errorReason(error);
  const known = reason ? reasonText(i18n, reason, options) : undefined;
  if (known) return known;
  if (reason === "RATE_LIMITED" || (isApiError(error) && error.code === Code.ResourceExhausted))
    return i18n.t("RATE_LIMITED", { ns: "errors" });
  if (!isApiError(error) || error.code === Code.Unavailable || error.code === Code.Unknown)
    return i18n.t("network", { ns: "errors" });
  return i18n.t("generic", { ns: "errors" });
}

/**
 * A duration limit in words: "60秒", "10分", "3時間" ("60 sec", "10 min",
 * "3 hr"). Up to two minutes count in seconds, as phones show them.
 */
export function durationLimit(i18n: I18n, seconds: number): string {
  if (seconds <= 120 || seconds % 60 !== 0)
    return i18n.t("limit.seconds", { ns: "errors", count: seconds });
  if (seconds % 3600 === 0) return i18n.t("limit.hours", { ns: "errors", count: seconds / 3600 });
  return i18n.t("limit.minutes", { ns: "errors", count: seconds / 60 });
}

/** What to say about an upload the server refused after processing. */
function assetFailureMessage(
  i18n: I18n,
  error: AssetFailedError,
  options: ErrorMessageOptions,
): string {
  if (error.reason === "MEDIA_TOO_LONG" && error.maxDurationSeconds > 0)
    return i18n.t("MEDIA_TOO_LONG_LIMIT", {
      ns: "errors",
      limit: durationLimit(i18n, error.maxDurationSeconds),
    });
  return reasonText(i18n, error.reason, options) ?? i18n.t("generic", { ns: "errors" });
}
