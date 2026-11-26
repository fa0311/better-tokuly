import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/** What else lives under `/plans` on the web, by its first segment. */
const WEB_PAGES = ["free", "polls", "countdowns", "anniversaries", "calendar"];

/**
 * The plans list, a new plan and every plan (`/plans/<id>`: an invitation,
 * a change, a reminder) open here. The rest of "あそぶ" — ひま, date polls,
 * countdowns and counters, the calendar — is the web's: it opens in the web
 * view, signed in.
 */
export default [
  { path: /^\/plans\/?$/, screen: () => "/plans" },
  { path: /^\/plans\/new$/, screen: () => "/plans/new" },
  {
    path: new RegExp(`^/plans/(?!(?:${WEB_PAGES.join("|")})$)([^/]+)$`),
    screen: (match) => `/plans/${match[1]}`,
  },
  { path: /^\/plans\/.+$/, takesLink: true, screen: (_match, link) => webScreenHref(link) },
] satisfies LinkTarget[];
