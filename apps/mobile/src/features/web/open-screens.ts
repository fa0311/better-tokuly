import { clearWebData } from "#/platform/web-data";

/** The web screens on the stack now, loading or shown. */
let open = 0;

/**
 * A web screen is on the stack until the function this returns is called.
 *
 * The web views' data is one store for the whole app (Android), so only the
 * last web screen to close empties it: one that closes late (it slides away
 * while the next one already loads, or it lay under a newer one) would
 * otherwise take the cookie the other one was just signed in with.
 */
export function webScreenOpened(): () => void {
  open += 1;
  return () => {
    open -= 1;
    void clearUnusedWebData();
  };
}

/**
 * Empties the web views' data unless a web screen is using it. Such a
 * screen started from empty data itself (`WebScreen`), so nothing older than
 * it is in there.
 */
export async function clearUnusedWebData(): Promise<void> {
  if (open === 0) await clearWebData();
}
