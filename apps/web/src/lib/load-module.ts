/**
 * Loads a piece of code split off the app's entry (a feature's chunk). A
 * load that fails, because the connection dropped right then, is tried again
 * each time the device comes back online, so a missing chunk leaves only its
 * own part off for a while: never the app's error screen.
 *
 * `loaded` runs once, with the module. The returned function stops trying.
 */
export function loadModule<T>(load: () => Promise<T>, loaded: (module: T) => void): () => void {
  let current = true;
  const attempt = () =>
    load().then(
      (module) => {
        if (!current) return;
        stop();
        loaded(module);
      },
      () => {},
    );
  const stop = () => {
    current = false;
    window.removeEventListener("online", attempt);
  };
  window.addEventListener("online", attempt);
  void attempt();
  return stop;
}
