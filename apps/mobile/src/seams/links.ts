import links from "#/generated/seams/links";

/**
 * Links: which screen of the app a path the server hands out opens (a
 * notification's target, the path a push carries).
 *
 * The paths are the web's (`/chat/<id>`, `/u/<handle>`). A feature
 * default-exports the ones it has a screen for from
 * `src/features/<area>/links.ts`:
 *
 *   export default [
 *     { path: /^\/chat\/([^/]+)$/, screen: (match) => `/chat/${match[1]}` },
 *   ] satisfies LinkTarget[];
 *
 * A path no feature claims is not in the app yet: the inbox opens it on the
 * web, a push opens the inbox (`features/notifications/target.ts`).
 */
export interface LinkTarget {
  /** Matches the whole pathname (no query, no fragment): anchor it with `^` and `$`. */
  path: RegExp;
  /** The route to open, ready for the router. */
  screen(match: RegExpMatchArray): string;
}

const TARGETS: LinkTarget[] = links.flat();

/** The app's screen for a server pathname, or `null` when there is none. */
export function screenFor(pathname: string, targets = TARGETS): string | null {
  for (const target of targets) {
    const match = pathname.match(target.path);
    if (match) return target.screen(match);
  }
  return null;
}
