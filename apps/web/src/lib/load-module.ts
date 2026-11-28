import { reloadIfRedeployed } from "./deploy";

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
