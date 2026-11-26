import type { LinkTarget } from "#/seams/links";

/** The settings the app has: the rest of `/settings` stays with the web (`features/web/links.ts`). */
export default [
  { path: /^\/settings$/, screen: () => "/settings" },
  { path: /^\/settings\/privacy$/, screen: () => "/settings/privacy" },
] satisfies LinkTarget[];
