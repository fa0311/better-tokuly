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
  /** Matches the whole pathname (the query and fragment are not part of it): anchor it with `^` and `$`. */
  path: RegExp;
  /**
   * The route to open, ready for the router. The link's query and fragment
   * (`?t=90`, `#c9`) follow it there, after a query of the route's own.
   */
  screen(match: RegExpMatchArray, link: string): string;
  /**
   * The route carries the whole link itself (`link`: the pathname with its
   * query and fragment), as the web view's does: nothing is added to it.
   */
  takesLink?: true;
}

const TARGETS: LinkTarget[] = links.flat();

/**
 * The app's screen for a link of the server (a pathname, with its query and
 * fragment if it has them), or `null` when there is none.
 */
export function screenFor(link: string, targets = TARGETS): string | null {
  const { pathname, query, fragment } = splitLink(link);
  for (const target of targets) {
    const match = pathname.match(target.path);
    if (!match) continue;
    const route = target.screen(match, link);
    if (target.takesLink) return route;
    const joined = query ? `${route}${route.includes("?") ? "&" : "?"}${query}` : route;
    return fragment && !joined.includes("#") ? `${joined}#${fragment}` : joined;
  }
  return null;
}

function splitLink(link: string): { pathname: string; query: string; fragment: string } {
  const hashAt = link.indexOf("#");
  const beforeHash = hashAt < 0 ? link : link.slice(0, hashAt);
  const fragment = hashAt < 0 ? "" : link.slice(hashAt + 1);
  const queryAt = beforeHash.indexOf("?");
  return {
    pathname: queryAt < 0 ? beforeHash : beforeHash.slice(0, queryAt),
    query: queryAt < 0 ? "" : beforeHash.slice(queryAt + 1),
    fragment,
  };
}
