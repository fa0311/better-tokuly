import type { LinkTarget } from "#/seams/links";

export default [
  { path: /^\/moments\/friends\/?$/, screen: () => "/moments/friends" },
  { path: /^\/moments\/?$/, screen: () => "/moments" },
] satisfies LinkTarget[];
