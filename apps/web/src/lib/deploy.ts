/**
 * A page left open across a deploy asks for chunks the server no longer has:
 * each deploy ships only its own build (`infra/docker/web.Dockerfile`), so
 * they answer 404 for good. Every chunk's name ends up in the entry's (each
 * importer names the chunks it loads), so a deploy that changed any chunk
 * changed the entry too: an entry the server no longer has means this page's
 * build is gone, and only a reload brings the code back.
 */

/** The entry this tab reloaded away from once (sessionStorage): it never reloads for it again. */
const RELOADED_FOR = "tokuly.reloadedFor";

let checking: Promise<void> | undefined;

/**
 * After a chunk failed to load: reloads the page into the current build when
 * this page's build is gone, once per build. When the server is unreachable
 * (offline) or still has the build, nothing happens: the load was unlucky,
 * and is tried again (`loadModule`).
 */
export function reloadIfRedeployed(): Promise<void> {
  checking ??= (async () => {
    const entry = entryUrl();
    if (!entry || !(await gone(entry))) return;
    try {
      if (sessionStorage.getItem(RELOADED_FOR) === entry) return;
      sessionStorage.setItem(RELOADED_FOR, entry);
    } catch {
      // Without storage a reload could not tell it already happened: no reload at all.
      return;
    }
    window.location.reload();
  })().finally(() => {
    checking = undefined;
  });
  return checking;
}

/**
 * Checks for a gone build whenever a chunk fails to load, wherever it was
 * imported (Vite says so with `vite:preloadError`). Returns what stops it.
 */
export function watchForRedeploy(): () => void {
  const check = () => void reloadIfRedeployed();
  window.addEventListener("vite:preloadError", check);
  return () => window.removeEventListener("vite:preloadError", check);
}

/** The entry chunk this page started from; none in development, where nothing is deployed. */
function entryUrl(): string | undefined {
  if (import.meta.env.DEV) return undefined;
  return document.querySelector<HTMLScriptElement>('script[type="module"][src]')?.src;
}

/**
 * Asks the server itself: a `HEAD` is never answered by the service worker
 * (`public/sw.js` handles GET only), and `no-store` skips the HTTP cache.
 */
async function gone(url: string): Promise<boolean> {
  const response = await fetch(url, { method: "HEAD", cache: "no-store" }).catch(() => undefined);
  return response?.status === 404;
}
