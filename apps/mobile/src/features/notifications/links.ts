import type { LinkTarget } from "#/seams/links";

export default [{ path: /^\/notifications$/, screen: () => "/inbox" }] satisfies LinkTarget[];
