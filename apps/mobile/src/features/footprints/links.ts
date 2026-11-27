import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * The own footprints open here (`/footprints`). Someone else's
 * (`/footprints?user=<id>`) and the scratch map itself stay on the web
 * screens, signed in, where the server decides what of them may be seen.
 */
export default [
  {
    path: /^\/footprints\/?$/,
    takesLink: true,
    screen: (_match, link) => (/[?&]user=/.test(link) ? webScreenHref(link) : "/footprints"),
  },
  { path: /^\/footprints\/.+$/, takesLink: true, screen: (_match, link) => webScreenHref(link) },
] satisfies LinkTarget[];
