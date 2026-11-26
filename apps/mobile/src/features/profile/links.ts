import type { LinkTarget } from "#/seams/links";

export default [
  { path: /^\/u\/([^/]+)$/, screen: (match) => `/u/${match[1]}` },
] satisfies LinkTarget[];
