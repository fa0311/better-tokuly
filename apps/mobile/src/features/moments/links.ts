import type { LinkTarget } from "#/seams/links";

export default [
  { path: /^\/moments\/calendar\/?$/, screen: () => "/moments/calendar" },
  { path: /^\/moments\/friends\/?$/, screen: () => "/moments/friends" },
  { path: /^\/moments\/?$/, screen: () => "/moments" },
] satisfies LinkTarget[];
