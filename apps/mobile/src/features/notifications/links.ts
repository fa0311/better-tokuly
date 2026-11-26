import type { LinkTarget } from "#/seams/links";

export default [
  { path: /^\/notifications$/, screen: () => "/inbox" },
  { path: /^\/notifications\/settings$/, screen: () => "/settings/notifications" },
] satisfies LinkTarget[];
