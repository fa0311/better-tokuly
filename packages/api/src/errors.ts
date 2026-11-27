import { Code, ConnectError } from "@connectrpc/connect";

/** Response header/trailer carrying the machine-readable error reason. */
const REASON_HEADER = "x-tokuly-error";

/**
 * The machine-readable reason of an API error (e.g. `HANDLE_TAKEN`), used to
 * pick a localized message. `undefined` for non-API errors.
 */
export function errorReason(error: unknown): string | undefined {
  if (!(error instanceof ConnectError)) return undefined;
  // The server sends the reason as a header and again as a trailer, and the
  // metadata joins repeated values ("HANDLE_TAKEN, HANDLE_TAKEN"): take one.
  const reason = error.metadata.get(REASON_HEADER)?.split(",")[0]?.trim();
  return reason || undefined;
}

export function isApiError(error: unknown): error is ConnectError {
  return error instanceof ConnectError;
}

/** True when the error means "not signed in" (as opposed to a rejected action). */
export function isUnauthenticated(error: unknown): boolean {
  return error instanceof ConnectError && error.code === Code.Unauthenticated;
}

/** True when the server asks the caller to slow down: a rate limit. */
export function isRateLimited(error: unknown): error is ConnectError {
  return error instanceof ConnectError && error.code === Code.ResourceExhausted;
}

/** How long the server asked to wait before trying again (`Retry-After`), in milliseconds. */
export function retryAfterMs(error: unknown): number | undefined {
  if (!(error instanceof ConnectError)) return undefined;
  const seconds = Number(error.metadata.get("retry-after")?.split(",")[0]);
  return Number.isFinite(seconds) && seconds >= 0 ? seconds * 1000 : undefined;
}

/** Reasons that mean the access token should be refreshed and the call retried. */
const STALE_TOKEN_REASONS = new Set(["TOKEN_EXPIRED", "TOKEN_INVALID"]);

export function isStaleToken(error: unknown): boolean {
  const reason = errorReason(error);
  return isUnauthenticated(error) && reason !== undefined && STALE_TOKEN_REASONS.has(reason);
}

/**
 * The account is under the minimum age (`ACCOUNT_TOO_YOUNG`): it answered
 * so, perhaps on another device, and reaches only signing in, its own
 * account and its deletion until that birthday.
 */
export function isAccountTooYoung(error: unknown): boolean {
  return (
    error instanceof ConnectError &&
    error.code === Code.PermissionDenied &&
    errorReason(error) === "ACCOUNT_TOO_YOUNG"
  );
}

export function isNotFound(error: unknown): boolean {
  return error instanceof ConnectError && error.code === Code.NotFound;
}

export { Code, ConnectError };
