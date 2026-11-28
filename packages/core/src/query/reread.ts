import { notifyManager, type Query, type QueryClient, type QueryKey } from "@tanstack/query-core";

/** The app's realtime connection, as far as reading again needs it (`RealtimeClient`). */
export interface WelcomingConnection {
  readonly connectionStatus: string;
  onFrame(listener: (frame: { body: { case?: string } }) => void): () => void;
}

/** After a reconnect, queries are read again at a random moment within this long. */
const REREAD_SPREAD_MS = 2_000;

/**
 * Reads the queries under `keys` again, and a read already in flight does
 * not count: it may have been answered before what the caller knows changed
 * (`docs/realtime-consistency.md`, rule 1). TanStack Query would join that
 * read when the query has no data yet, so it is cancelled and replaced here;
 * whoever waited on it gets the new answer. Queries on screen are read at
 * once, the others are marked stale and read when they are next shown.
 * `covered` leaves out queries that read themselves again (a topic's
 * `onSubscribed`).
 */
export function reread(
  queryClient: QueryClient,
  keys: readonly QueryKey[],
  covered: (queryKey: QueryKey) => boolean = () => false,
): Promise<void> {
  const reads: Promise<unknown>[] = [];
  const seen = new Set<Query>();
  notifyManager.batch(() => {
    for (const queryKey of keys)
      for (const query of queryClient.getQueryCache().findAll({ queryKey })) {
        if (seen.has(query) || covered(query.queryKey)) continue;
        seen.add(query);
        query.invalidate();
        // Not `refetchQueries`: it passes over a first read nobody watches yet (a route's loader).
        if (query.state.fetchStatus !== "idle") void query.cancel({ silent: true });
        else if (!query.isActive() || query.isStatic()) continue;
        reads.push(query.fetch().catch(() => {}));
      }
  });
  return Promise.all(reads).then(() => {});
}

/**
 * Calls {@link reread} for `keys` on every Welcome of `connection`, the
 * first one too: a read that started before the user's subscription was in
 * place may have missed a push, and pushes sent while the connection was
 * down are lost. The first Welcome reads at once; one after a reconnect at a
 * random moment within two seconds, so that a restarted gateway's audience
 * does not read again all at once. Returns a function that stops it.
 *
 * Register it before the connection's first Welcome can arrive: in the
 * effect that calls `client.start()`, before that call (the background
 * features do). Registered later, it treats the next Welcome as a reconnect
 * and has missed the first: reads of `keys` begun before then are kept. A
 * screen registering while connected is fine for queries it starts itself;
 * one whose keys may already hold such reads calls `reread` once as well.
 */
export function rereadOnWelcome(
  queryClient: QueryClient,
  connection: WelcomingConnection,
  keys: readonly QueryKey[],
  covered?: (queryKey: QueryKey) => boolean,
): () => void {
  let reconnect = connection.connectionStatus === "connected";
  let timer: ReturnType<typeof setTimeout> | undefined;
  const off = connection.onFrame((frame) => {
    if (frame.body.case !== "welcome") return;
    clearTimeout(timer);
    if (reconnect)
      timer = setTimeout(
        () => void reread(queryClient, keys, covered),
        Math.random() * REREAD_SPREAD_MS,
      );
    else void reread(queryClient, keys, covered);
    reconnect = true;
  });
  return () => {
    off();
    clearTimeout(timer);
  };
}
