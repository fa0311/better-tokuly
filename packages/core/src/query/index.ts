import { Code, ConnectError } from "@tokuly/api";

/** Errors that will not get better by asking again. */
const FINAL_CODES = new Set([
  Code.NotFound,
  Code.PermissionDenied,
  Code.Unauthenticated,
  Code.InvalidArgument,
  Code.FailedPrecondition,
  Code.AlreadyExists,
  Code.Unimplemented,
]);

/** The query retry rule of every app: twice at most, never for an answer that will not change. */
export function shouldRetry(failures: number, error: unknown): boolean {
  return failures < 2 && !(error instanceof ConnectError && FINAL_CODES.has(error.code));
}
