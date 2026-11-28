import {
  hashKey,
  type QueryClient,
  type QueryFunction,
  type QueryFunctionContext,
  type QueryKey,
} from "@tanstack/query-core";
import { pushesDuringReadByKey } from "./pushes";

/**
 * Rule 2 of `docs/realtime-consistency.md`: a push never loses to an older
 * read. A query reads with `{ ...options, persister: journal }`, plain or
 * infinite, and every push goes through `pushTo` (or `pushToAll`); a read
 * that is not the query's own and writes the cache goes through
 * `pushAfter`. Nothing else is wired by hand.
 *
 * `pushTo` changes the cached data now, and keeps the update for every read
 * of that query in flight: its answer may have been made before the push,
 * so the update is applied to it again when it lands (see `pushesDuringRead`).
 * Removals go through `pushTo` too, so an older read cannot bring back what
 * a push took away. Updates must therefore be idempotent: add by ID (replace
 * what is there), remove by ID, keep the higher version.
 *
 * An update sees what the query holds: the value of a plain query, the
 * `{ pages, pageParams }` of an infinite one. A query that holds `null` or
 * `undefined` has nothing to change: updates see the data only, and leave an
 * empty answer as it is. An update may return `null` (what the query held is
 * gone: a revoked widget, a plan the viewer was removed from), and an older
 * read cannot bring it back.
 */

/** One journal per query client (each app has one; tests make their own), by query hash. */
const journals = new WeakMap<QueryClient, ReturnType<typeof pushesDuringReadByKey<unknown>>>();

function journalOf(queryClient: QueryClient) {
  let journal = journals.get(queryClient);
  if (!journal) {
    journal = pushesDuringReadByKey<unknown>();
    journals.set(queryClient, journal);
  }
  return journal;
}

/** A pushed change: what the query holds next, or `null` when what it held is gone (where `T` allows it). */
export type Update<T> = (current: NonNullable<T>) => NonNullable<T> | Extract<T, null>;

const replay =
  <T>(update: Update<T>) =>
  (state: unknown) =>
    state === null || state === undefined ? state : update(state as NonNullable<T>);

/**
 * The read side, as a TanStack Query `persister`: `{ ...options, persister:
 * journal }`. It wraps the whole read — a plain query's function, or every
 * page an infinite query fetches (all of them again, or the one page
 * `fetchNextPage` adds to what was held when it started) — so that what
 * `pushTo` pushed meanwhile is applied to the answer.
 *
 * A read that is cancelled (replaced by `reread`, or its last observer
 * left) stops collecting pushes at once. Pass the query's `signal` to the
 * request too, so that the request ends as well. TanStack then cancels a
 * read nobody watches any more instead of letting it finish: the query
 * keeps what it held, marked as it was, and reads again when next shown.
 */
export const journal: QueryJournal = async (queryFn, context) => {
  const read = journalOf(context.client).begin(hashKey(context.queryKey));
  const signal = context.signal;
  if (signal.aborted) read.cancel();
  else signal.addEventListener("abort", () => read.cancel(), { once: true });
  let answer: unknown;
  try {
    answer = await (queryFn as QueryFunction)(context);
  } catch (error) {
    read.cancel();
    throw error;
  }
  return read.finish(answer) as never;
};

/**
 * The type `persister` takes, written so that it adds nothing to what
 * TanStack Query infers from `queryFn`: the answer passes through unchanged
 * but for the pushes replayed on it.
 */
type QueryJournal = (
  queryFn: (...args: never[]) => unknown,
  context: QueryFunctionContext,
) => Promise<never>;

/**
 * Applies a pushed change to the query under `key` (exactly that key) now and to every read of it in flight.
 * A push is not a read: the query keeps its age and, when it was marked to be read again, stays marked.
 */
export function pushTo<T>(queryClient: QueryClient, key: QueryKey, update: Update<T>): void {
  const change = replay(update);
  journalOf(queryClient).record(hashKey(key), change);
  write(queryClient, key, change, "pushed");
}

/** Writes `change` into the cached query: `read` makes it as new as a read would, `pushed` keeps its age. */
function write(
  queryClient: QueryClient,
  key: QueryKey,
  change: (state: unknown) => unknown,
  as: "read" | "pushed",
): void {
  const query = queryClient.getQueryCache().find({ queryKey: key, exact: true });
  if (!query) return;
  const { data, dataUpdatedAt, isInvalidated } = query.state;
  const next = change(data);
  if (next === data) return;
  queryClient.setQueryData(key, next, as === "pushed" ? { updatedAt: dataUpdatedAt } : undefined);
  if (isInvalidated) query.invalidate();
}

/**
 * `pushTo` for every query under `prefix`: one list held under several keys
 * (a filter, a source, a search). The queries under it must all hold the
 * same kind of data. A query is in the cache from the moment its first read
 * starts, so no read in flight is left out.
 */
export function pushToAll<T>(queryClient: QueryClient, prefix: QueryKey, update: Update<T>): void {
  for (const query of queryClient.getQueryCache().findAll({ queryKey: prefix }))
    pushTo(queryClient, query.queryKey, update);
}

/**
 * A read that is not the query's own but writes it: the comment fetched
 * after `commentAdded`, the answer to a mutation. `read` returns the change
 * its answer makes (or nothing). Pushes to `key` that arrive while it runs
 * may be newer than that answer, so the change is pushed as "the answer,
 * then what was pushed since": to the cache and, like any push, to the
 * query's own reads in flight, where the pushes since are replayed after it
 * again (a removal that followed the add stays a removal). Being a read, it
 * makes the query as new as its answer.
 */
export async function pushAfter<T>(
  queryClient: QueryClient,
  key: QueryKey,
  read: () => Promise<Update<T> | undefined>,
): Promise<void> {
  const since = journalOf(queryClient).begin(hashKey(key));
  let answer: Update<T> | undefined;
  try {
    answer = await read();
  } finally {
    // Stops collecting (before the push below, which would otherwise collect itself); what was
    // collected stays, and `finish` applies it as often as the change is applied.
    since.cancel();
  }
  if (!answer) return;
  const update = answer;
  const change = replay<T>((current) => since.finish(update(current)) as ReturnType<Update<T>>);
  journalOf(queryClient).record(hashKey(key), change);
  write(queryClient, key, change, "read");
}
