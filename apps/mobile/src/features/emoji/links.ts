import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * The emoji paths: the catalogue, the deck and an emoji's page open here;
 * sending one in and one's submissions (`emoji.submission_*`) are still the
 * web's.
 */
export default [
  { path: /^\/emoji(?:\/my-deck)?\/?$/, screen: () => "/emoji" },
  {
    path: /^\/emoji\/(?:new-emoji|submissions)(?:\/.*)?$/,
    takesLink: true,
    screen: (_match, link) => webScreenHref(link),
  },
  { path: /^\/emoji\/([^/]+)$/, screen: (match) => `/emoji/${match[1]}` },
] satisfies LinkTarget[];
