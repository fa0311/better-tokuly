import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * The wallet's front page, its history and the list of purchases open
 * here. Buying coins, one purchase (`/wallet/purchases/<id>`: paying,
 * checking, cancelling), earnings and payouts stay on the web screens,
 * signed in.
 */
export default [
  { path: /^\/wallet\/?$/, screen: () => "/wallet" },
  { path: /^\/wallet\/history\/?$/, screen: () => "/wallet/history" },
  { path: /^\/wallet\/purchases\/?$/, screen: () => "/wallet/purchases" },
  { path: /^\/wallet\/.+$/, takesLink: true, screen: (_match, link) => webScreenHref(link) },
] satisfies LinkTarget[];
