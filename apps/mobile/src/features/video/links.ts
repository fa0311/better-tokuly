import type { LinkTarget } from "#/seams/links";

/**
 * The server's video paths (a new upload of a subscribed channel, a reply
 * to a comment, a shared link) open the app's own screens. The studio and
 * watch parties stay on the web (`features/web/links.ts`).
 */
export default [
  { path: /^\/watch\/?$/, screen: () => "/watch" },
  { path: /^\/watch\/(history|channels)$/, screen: (match) => `/watch/${match[1]}` },
  { path: /^\/watch\/playlists$/, screen: () => "/watch/library" },
  { path: /^\/watch\/([A-Za-z0-9_-]+)$/, screen: (match) => `/watch/${match[1]}` },
  { path: /^\/c\/([^/]+)(?:\/[^/]+)?$/, screen: (match) => `/c/${match[1]}` },
  { path: /^\/playlist\/([^/]+)$/, screen: (match) => `/playlist/${match[1]}` },
] satisfies LinkTarget[];
