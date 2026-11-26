import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * The live list and every stream (`/live/<id>`, its UUID or short ID: a
 * `live.reminder` push, a friend going live, a shared link) open here.
 * What else lives under `/live` — the gift history, the clip editor — is
 * the web's: it opens in the web view, signed in.
 */
export default [
  { path: /^\/live\/?$/, screen: () => "/live" },
  { path: /^\/live\/(?!(?:gifts|clips)$)([^/]+)$/, screen: (match) => `/live/${match[1]}` },
  { path: /^\/live\/.+$/, takesLink: true, screen: (_match, link) => webScreenHref(link) },
] satisfies LinkTarget[];
