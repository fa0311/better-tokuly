import { errorReason } from "@tokuly/api";
import { errorMessage as sharedErrorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";

/**
 * The API's reason code for an error (`INVALID_TOTP`, `HANDLE_TAKEN`, ...).
 *
 * The server sends the reason twice, as a response header and as a Connect
 * trailer, and the client library joins the two into "X, X". Until that is
 * settled in one place for the whole app, the settings read the first one.
 */
export function reasonOf(error: unknown): string | undefined {
  return errorReason(error)?.split(",")[0]?.trim() || undefined;
}

/** The text to show a person for a failed request: the reason's own message when there is one. */
export function errorMessage(error: unknown): string {
  const reason = reasonOf(error);
  if (reason && i18n.exists(reason, { ns: "errors" })) return i18n.t(reason, { ns: "errors" });
  if (reason && i18n.exists(`errors.${reason}`, { ns: "settings" })) return i18n.t(`errors.${reason}`, { ns: "settings" });
  return sharedErrorMessage(error);
}
