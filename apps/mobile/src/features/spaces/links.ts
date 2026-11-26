import type { LinkTarget } from "#/seams/links";

/**
 * The rooms list, every room (`spaces.started`, `spaces.reminder`,
 * `spaces.invited`) and every hangout (`/hangouts/<id>`, the server's
 * address for one, and `/spaces/hangouts/<id>`, the web's) open here.
 */
export default [
  { path: /^\/spaces\/?$/, screen: () => "/spaces" },
  {
    path: /^\/(?:spaces\/)?hangouts\/([^/]+)$/,
    screen: (match) => `/spaces/hangouts/${match[1]}`,
  },
  { path: /^\/spaces\/([^/]+)$/, screen: (match) => `/spaces/${match[1]}` },
] satisfies LinkTarget[];
