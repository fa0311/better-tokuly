import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * The server's notes paths: what the phone has opens here; the rest of
 * `/notes` (channels, editing lists and antennas, mutes) stays the web's,
 * signed in, in the web view.
 */
export default [
  { path: /^\/notes$/, screen: () => "/notes" },
  { path: /^\/notes\/drafts$/, screen: () => "/notes/drafts" },
  { path: /^\/notes\/clips$/, screen: () => "/notes/clips" },
  { path: /^\/notes\/clips\/([^/]+)$/, screen: (match) => `/notes/clips/${match[1]}` },
  { path: /^\/notes\/tags\/([^/]+)$/, screen: (match) => `/notes/tag/${match[1]}` },
  { path: /^\/notes\/lists\/([^/]+)$/, screen: (match) => `/notes/list/${match[1]}` },
  { path: /^\/notes\/antennas\/([^/]+)$/, screen: (match) => `/notes/antenna/${match[1]}` },
  {
    path: /^\/notes\/(?!channels$|lists$|antennas$)([^/]+)$/,
    screen: (match) => `/notes/${match[1]}`,
  },
  { path: /^\/notes(?:\/.*)?$/, screen: (match) => webScreenHref(match[0]) },
] satisfies LinkTarget[];
