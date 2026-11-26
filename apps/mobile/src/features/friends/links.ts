import type { LinkTarget } from "#/seams/links";

/** Everything under the web's `/friends` (requests, suggestions) is the one friends tab here. */
export default [
  { path: /^\/invite\/([^/]+)$/, screen: (match) => `/invite/${match[1]}` },
  { path: /^\/friends(\/.*)?$/, screen: () => "/friends" },
] satisfies LinkTarget[];
