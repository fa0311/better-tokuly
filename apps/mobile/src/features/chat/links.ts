import type { LinkTarget } from "#/seams/links";

export default [
  { path: /^\/chat\/([^/]+)$/, screen: (match) => `/chat/${match[1]}` },
] satisfies LinkTarget[];
