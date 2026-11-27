import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * Places open here: the list (`/places`) and a place (`/places/<id>`).
 * Making and editing one (`/places/new`, `/places/<id>/edit`) stays on the
 * web screens, signed in.
 */
export default [
  { path: /^\/places\/?$/, screen: () => "/places" },
  { path: /^\/places\/new\/?$/, takesLink: true, screen: (_match, link) => webScreenHref(link) },
  { path: /^\/places\/([^/]+)\/?$/, screen: (match) => `/places/${match[1]}` },
  { path: /^\/places\/.+$/, takesLink: true, screen: (_match, link) => webScreenHref(link) },
] satisfies LinkTarget[];
