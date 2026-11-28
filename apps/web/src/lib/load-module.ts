import { NetworkError } from "@tokuly/core/errors";
import { reloadIfRedeployed } from "./deploy";

/**
 * Code split off the app's entry loads through here, or through what is built
 * on it (`onDemand`, `onFramesLoading`): a bare `import()` has no way out of
 * a failure, so it fails lint (`lint/no-bare-import.grit`).
 */

/**
 * Loads a piece of code split off the app's entry (a feature's chunk). A
 * load that fails, because the connection dropped right then, is tried again
 * each time the device comes back online (also when it came back while that
 * load was still on its way), so a missing chunk leaves only its own part off
 * for a while: never the app's error screen. A chunk gone with a deploy never
 * comes back: then the page reloads into the new build (`reloadIfRedeployed`).
 *
 * `loaded` runs once, with the module. The returned function stops trying.
 */
export function loadModule<T>(load: () => Promise<T>, loaded: (module: T) => void): () => void {
  let current = true;
  let loading = false;
  let onlineSince = false;
  const attempt = () => {
    if (loading) {
      onlineSince = true;
      return;
    }
    loading = true;
    onlineSince = false;
    load().then(
      (module) => {
        if (!current) return;
        stop();
        loaded(module);
      },
      () => {
        loading = false;
        void reloadIfRedeployed();
        if (current && onlineSince) attempt();
      },
    );
  };
  const stop = () => {
    current = false;
    window.removeEventListener("online", attempt);
  };
  window.addEventListener("online", attempt);
  attempt();
  return stop;
}

/**
 * Loads code something waits on right now: a player about to play, what a tap
 * opens. A load that fails, after the check for a gone build
 * (`reloadIfRedeployed`), rejects with a `NetworkError`, so `errorMessage`
 * asks to check the connection; nothing keeps the failure, and the next call
 * loads again. Code nobody waits on loads with `loadModule`, which tries
 * again by itself.
 */
export function loadNow<T>(load: () => Promise<T>): Promise<T> {
  return load().catch((error: unknown) => {
    void reloadIfRedeployed();
    throw new NetworkError("code failed to load", { cause: error });
  });
}
