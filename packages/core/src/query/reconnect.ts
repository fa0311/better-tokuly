import type { QueryClient, QueryKey } from "@tanstack/query-core";

/** The app's realtime connection, as far as reading again needs it (`RealtimeClient`). */
export interface ReconnectingConnection {
  readonly connectionStatus: string;
  onFrame(listener: (frame: { body: { case?: string } }) => void): () => void;
}

/** After a reconnect, lists are read again at a random moment within this long. */
const REREAD_SPREAD_MS = 2_000;

/**
 * Calls `reconnected` on every Welcome of `connection` but the first one:
 * pushes sent while the connection was down are gone, and whatever they
 * would have changed is read again there. Returns a function that stops it.
 */
export function afterReconnect(
  connection: ReconnectingConnection,
  reconnected: () => void,
): () => void {
  let connectedBefore = connection.connectionStatus === "connected";
  return connection.onFrame((frame) => {
    if (frame.body.case !== "welcome") return;
    const again = connectedBefore;
    connectedBefore = true;
    if (again) reconnected();
  });
}

/**
 * Re-reads the lists under `root` that are on screen (queries with an
 * observer), once after every reconnect, at a random moment within two
 * seconds so that a restarted gateway's audience does not read again all at
 * once. `covered` leaves out queries a topic subscription already reads
 * again itself (`SubscribeOptions.onSubscribed`). Returns a function that
 * stops it.
 */
export function rereadListsAfterReconnect(
  queryClient: QueryClient,
  connection: ReconnectingConnection,
  root: QueryKey,
  covered: (queryKey: QueryKey) => boolean = () => false,
): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const off = afterReconnect(connection, () => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      void queryClient.invalidateQueries({
        queryKey: root,
        refetchType: "active",
        predicate: (query) => query.getObserversCount() > 0 && !covered(query.queryKey),
      });
    }, Math.random() * REREAD_SPREAD_MS);
  });
  return () => {
    off();
    clearTimeout(timer);
  };
}
