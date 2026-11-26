import type { LinkTarget } from "#/seams/links";

/** A result page of the web (`/search?q=…&kind=…`) is the search screen here; its query follows it. */
export default [{ path: /^\/search$/, screen: () => "/search" }] satisfies LinkTarget[];
