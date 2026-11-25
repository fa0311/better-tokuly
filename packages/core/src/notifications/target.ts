/** Where a server-sent "path in the app" leads. */
export type Target =
  /** A page of the app: path, search and hash, ready for its router. */
  | { type: "internal"; href: string }
  /** Somewhere else on the web. */
  | { type: "external"; url: string };

/**
 * Reads a notification's `path` (or any target the server hands out:
 * `/chat/42`, `/u/alice?tab=notes`, a full URL of the app's own server,
 * occasionally an outside link). `origin` is the app's server. `null` when
 * there is nowhere to go: an empty path, or a scheme that is not http(s).
 * Each app maps the internal paths onto its own screens.
 */
export function resolveTarget(path: string, origin: string): Target | null {
  const trimmed = path.trim();
  if (!trimmed) return null;
  let url: URL;
  try {
    url = new URL(trimmed, origin);
  } catch {
    return null;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return null;
  if (url.origin !== new URL(origin).origin) return { type: "external", url: url.href };
  const pathname = url.pathname.length > 1 ? url.pathname.replace(/\/+$/, "") : url.pathname;
  return { type: "internal", href: pathname + url.search + url.hash };
}
