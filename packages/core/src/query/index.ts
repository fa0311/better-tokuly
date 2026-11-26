import type { DescService } from "@bufbuild/protobuf";
import type { Client } from "@connectrpc/connect";
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

export { type PushesDuringRead, pushesDuringRead, pushesDuringReadByKey } from "./pushes";
