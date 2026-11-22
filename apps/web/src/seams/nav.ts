/**
 * Which primary destination a page belongs to, so the tab bar and the rail
 * highlight the right item on nested routes.
 *
 * The primary items are the app's decision (`components/app-shell.tsx`); a
 * feature says which of them its routes sit under by default-exporting a
 * claim from `src/features/<area>/nav.ts`:
 *
 *   export default { primary: "notes", paths: ["/notes", "/tag", "/stories"] } satisfies NavClaim;
 *
 * A path claims itself and everything below it. Pages nobody claims belong
 * to the hub ("me"): that is where people open them from.
 */
export type PrimaryNav = "map" | "chat" | "notes" | "watch" | "search" | "notifications" | "me";

export interface NavClaim {
  primary: PrimaryNav;
  /** Path prefixes, each starting with "/": "/c" covers "/c/tokuly/videos". */
  paths: string[];
}

const modules = import.meta.glob<{ default: NavClaim | NavClaim[] }>("../features/*/nav.ts", {
  eager: true,
});

// The longest prefix wins, so "/watch/history" could sit elsewhere than "/watch".
const CLAIMS = Object.values(modules)
  .flatMap((module) => module.default)
  .flatMap((claim) => claim.paths.map((path) => ({ path, primary: claim.primary })))
  .sort((a, b) => b.path.length - a.path.length);

/** The primary destination a pathname belongs to. */
export function primaryNavOf(pathname: string, claims = CLAIMS): PrimaryNav {
  const claim = claims.find(({ path }) => pathname === path || pathname.startsWith(`${path}/`));
  return claim?.primary ?? "me";
}
