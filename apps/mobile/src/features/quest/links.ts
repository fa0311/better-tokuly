import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * The quest home (level and missions: a mission completed, a reward
 * waiting) opens here. Badges, streaks, rankings, the XP guide, the
 * showcase, a player's quest page and the settings stay on the web
 * screens, signed in.
 */
export default [
  { path: /^\/quest\/?$/, screen: () => "/quest" },
  { path: /^\/quest\/.+$/, takesLink: true, screen: (_match, link) => webScreenHref(link) },
] satisfies LinkTarget[];
