import { webScreenHref } from "#/features/web/handoff";
import type { LinkTarget } from "#/seams/links";

/**
 * The question box's paths: the hub, the inbox (`ask.question`), a question
 * (`ask.answered`), a box and the questions sent open here; a shared box
 * link (`/q/<handle>`) and a profile's box card (`/u/<handle>/ask`) open the
 * box in the app. Prompts, polls, flames
 * (`ask.prompt`, `ask.prompt_response`, `ask.flame`) and the settings are
 * still the web's.
 */
export default [
  { path: /^\/ask\/?$/, screen: () => "/ask" },
  { path: /^\/ask\/inbox$/, screen: () => "/ask/inbox" },
  { path: /^\/ask\/sent$/, screen: () => "/ask/sent" },
  { path: /^\/ask\/q\/([^/]+)$/, screen: (match) => `/ask/q/${match[1]}` },
  { path: /^\/(?:ask\/u|q)\/([^/]+)$/, screen: (match) => `/ask/u/${match[1]}` },
  { path: /^\/u\/([^/]+)\/ask$/, screen: (match) => `/ask/u/${match[1]}` },
  {
    path: /^\/ask\/(?:prompts|polls|flames|settings)(?:\/.*)?$/,
    takesLink: true,
    screen: (_match, link) => webScreenHref(link),
  },
] satisfies LinkTarget[];
