import { isApiError } from "@tokuly/api";
import { errorMessage as sharedErrorMessage } from "#/lib/errors";

/** Response header (and trailer) carrying the API's machine-readable error reason. */
const REASON_HEADER = "x-tokuly-error";

/**
 * `errorMessage` from `#/lib/errors`, for errors whose reason arrived twice.
 *
 * The server sends the reason both as a header and as a trailer, and the
 * Connect client merges the two into one value ("FLAG_EXISTS, FLAG_EXISTS"),
 * which no translation matches — every API error would read as the generic
 * apology. Until `errorReason` in `@tokuly/api` takes the first value, the
 * duplicate is dropped here before the shared lookup runs. Once that is fixed
 * this file can go and its callers can import `#/lib/errors` again.
 */
export function errorMessage(error: unknown): string {
  if (isApiError(error)) {
    const [reason] = (error.metadata.get(REASON_HEADER) ?? "").split(",");
    if (reason?.trim()) error.metadata.set(REASON_HEADER, reason.trim());
  }
  return sharedErrorMessage(error);
}
