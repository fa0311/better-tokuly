import type { DescService } from "@bufbuild/protobuf";
import type { Client } from "@connectrpc/connect";
import { Code, ConnectError, isRateLimited, retryAfterMs } from "@tokuly/api";

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

/** The longest wait a rate-limited query is asked again after; longer, the screen shows the error. */
const RATE_LIMIT_WAIT_MS = 10_000;

/**
 * The query retry rule of every app: twice at most, never for an answer that
 * will not change. A rate-limited query is asked again once, and only when
 * the server named a short wait (see `retryDelay`): asking blindly would
 * only add to the count.
 */
export function shouldRetry(failures: number, error: unknown): boolean {
  if (isRateLimited(error)) {
    const wait = retryAfterMs(error);
    return failures < 1 && wait !== undefined && wait <= RATE_LIMIT_WAIT_MS;
  }
  return failures < 2 && !(error instanceof ConnectError && FINAL_CODES.has(error.code));
}

/** How long a query waits before it is asked again: what the server named, or TanStack Query's own backoff. */
export function retryDelay(failures: number, error: unknown): number {
  return (isRateLimited(error) && retryAfterMs(error)) || Math.min(1000 * 2 ** failures, 30_000);
}

/**
 * Where a query gets its typed API client: a function, so query options can
 * be built while rendering before the API exists (a server render), and the
 * client is only asked for when the query runs. Each app passes its own.
 */
export type ClientSource<S extends DescService> = () => Client<S>;

/** `getNextPageParam` for the API's cursor pages: the next cursor, or the end. */
export function nextCursor(last: { page?: { nextCursor: string } }): string | undefined {
  return last.page?.nextCursor || undefined;
}

export { journal, pushAfter, pushTo, pushToAll, type Update } from "./journal";
export { reread, rereadOnWelcome, type WelcomingConnection } from "./reread";
