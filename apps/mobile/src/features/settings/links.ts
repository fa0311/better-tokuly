import type { LinkTarget } from "#/seams/links";

/** The settings the app has: the rest of `/settings` stays with the web (`features/web/links.ts`). */
export default [
  { path: /^\/settings$/, screen: () => "/settings" },
  { path: /^\/settings\/profile$/, screen: () => "/settings/profile" },
  { path: /^\/settings\/privacy$/, screen: () => "/settings/privacy" },
  { path: /^\/settings\/security$/, screen: () => "/settings/security" },
] satisfies LinkTarget[];
