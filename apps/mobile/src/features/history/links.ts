import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * The history opens here: the calendar (`/history`) and a day
 * (`/history/<YYYY-MM-DD>`), where the day's recap leads. Anything else under
 * `/history` stays on the web screens, signed in.
 */
export default [
  { path: /^\/history\/?$/, screen: () => "/history" },
  { path: /^\/history\/(\d{4}-\d{2}-\d{2})\/?$/, screen: (match) => `/history/${match[1]}` },
  { path: /^\/history\/.+$/, takesLink: true, screen: (_match, link) => webScreenHref(link) },
] satisfies LinkTarget[];
