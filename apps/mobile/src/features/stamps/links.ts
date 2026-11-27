import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * The stamp store's paths: the store and its shelves, a pack and the
 * library open here. The studio and a creator's page (making, selling) are
 * still the web's.
 */
export default [
  { path: /^\/stamps(?:\/browse)?\/?$/, screen: () => "/stamps" },
  { path: /^\/stamps\/library$/, screen: () => "/stamps/library" },
  {
    path: /^\/stamps\/(?:studio|creator)(?:\/.*)?$/,
    takesLink: true,
    screen: (_match, link) => webScreenHref(link),
  },
  { path: /^\/stamps\/([^/]+)$/, screen: (match) => `/stamps/${match[1]}` },
] satisfies LinkTarget[];
