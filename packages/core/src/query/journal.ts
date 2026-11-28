import { hashKey, type QueryClient, type QueryFunction, type QueryKey } from "@tanstack/query-core";
import { pushesDuringReadByKey } from "./pushes";

/**
 * Rule 2 of `docs/realtime-consistency.md` for a query: a push never loses
 * to an older read. A feature reads with `journaled(queryFn)` and changes
 * what a push changed with `pushTo(queryClient, key, update)`; nothing else
 * is wired by hand.
 *
 * `pushTo` changes the cached data now, and keeps the update for every read
 * of that query in flight: its answer may have been made before the push,
 * so the update is applied to it again when it lands (see `pushesDuringRead`).
 * Removals go through `pushTo` too, so an older read cannot bring back what
 * a push took away. Updates must therefore be idempotent: add by ID (replace
 * what is there), remove by ID, keep the higher version.
 *
 * A query that holds `null` or `undefined` has nothing to change: updates
 * see the data only, and leave an empty answer as it is. An update may
 * return `null` (what the query held is gone: a revoked widget, a plan the
 * viewer was removed from), and an older read cannot bring it back.
 *
 * For plain queries only: an infinite query calls its query function once
 * per page, so a page's answer is not what `pushTo` updates.
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
type Update<T> = (current: NonNullable<T>) => NonNullable<T> | Extract<T, null>;

const replay =
  <T>(update: Update<T>) =>
  (state: unknown) =>
    state === null || state === undefined ? state : update(state as NonNullable<T>);

/** Wraps a query function so that what `pushTo` pushed while it ran is applied to its answer. */
export function journaled<T, TKey extends QueryKey = QueryKey>(
  queryFn: QueryFunction<T, TKey>,
): QueryFunction<T, TKey> {
  return async (context) => {
    if ("pageParam" in context)
      throw new Error(`journaled() reads a whole query, not a page: ${String(context.queryKey)}`);
    const read = journalOf(context.client).begin(hashKey(context.queryKey));
    let answer: T;
    try {
      answer = await queryFn(context);
    } catch (error) {
      read.cancel();
      throw error;
    }
    return read.finish(answer) as T;
  };
}

/** Applies a pushed change to the query under `key` (exactly that key) now and to every read of it in flight. */
export function pushTo<T>(queryClient: QueryClient, key: QueryKey, update: Update<T>): void {
  const change = replay(update);
  journalOf(queryClient).record(hashKey(key), change);
  queryClient.setQueryData<T>(key, (current) => change(current) as T | undefined);
}
