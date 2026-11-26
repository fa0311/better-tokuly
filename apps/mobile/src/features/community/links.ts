import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * Communities open here: the list, a community, a thread, its members and
 * an invite (`/join/<code>`; `/join/party/…` is another area's). The
 * domain publishes its pages as `/c/…`; notifications reach these through
 * `notificationPath` (`@tokuly/core/notifications`). Events and the staff's
 * pages stay on the web screens, signed in.
 */
export default [
  { path: /^\/communities\/?$/, screen: () => "/communities" },
  { path: /^\/g\/([^/]+)\/?$/, screen: (match) => `/g/${match[1]}` },
  { path: /^\/g\/([^/]+)\/t\/([^/]+)$/, screen: (match) => `/g/${match[1]}/t/${match[2]}` },
  { path: /^\/g\/([^/]+)\/members$/, screen: (match) => `/g/${match[1]}/members` },
  { path: /^\/g\/[^/]+\/.+$/, takesLink: true, screen: (_match, link) => webScreenHref(link) },
  { path: /^\/join\/(?!party$)([^/]+)$/, screen: (match) => `/g/join/${match[1]}` },
] satisfies LinkTarget[];
