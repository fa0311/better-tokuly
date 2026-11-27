import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * The quest home (missions) and the badges open here: the collection and
 * one badge (`/quest/badges/<key>`: a badge unlocked, a badge a friend
 * sent). Streaks, rankings, the XP guide, the showcase, a player's quest
 * page and the settings stay on the web screens, signed in.
 */
export default [
  { path: /^\/quest\/?$/, screen: () => "/quest" },
  { path: /^\/quest\/badges\/?$/, screen: () => "/quest/badges" },
  { path: /^\/quest\/badges\/([^/]+)$/, screen: (match) => `/quest/badges/${match[1]}` },
  { path: /^\/quest\/.+$/, takesLink: true, screen: (_match, link) => webScreenHref(link) },
] satisfies LinkTarget[];
