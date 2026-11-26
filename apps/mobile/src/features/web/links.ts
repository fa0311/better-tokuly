import type { LinkTarget } from "#/seams/links";
import { webScreenHref } from "./handoff";

/**
 * The web app's areas the phone has no screens for yet: a notification, a
 * push or a link into one of them opens it in the web view, signed in,
 * instead of the inbox. (Areas the app has — chat, profiles, friends, the
 * map, shorts, stories, videos — are claimed by their own features; none is listed
 * here.) The first path segments are those the server's hand-off accepts
 * (identity's `handoff::WEB_ROUTES`).
 */
const WEB_ONLY = [
  "activity",
  "admin",
  "announcements",
  "ask",
  "bookmarks",
  "circles",
  "clip",
  "communities",
  "developers",
  "emoji",
  "footprints",
  "g",
  "hangouts",
  "history",
  "live",
  "news",
  "notes",
  "party",
  "places",
  "plans",
  "quest",
  "recap",
  "school",
  "search",
  "settings",
  "spaces",
  "stamps",
  "studio",
  "wallet",
];

export default [
  {
    path: new RegExp(`^/(?:${WEB_ONLY.join("|")})(?:/.*)?$`),
    screen: (match) => webScreenHref(match[0]),
  },
] satisfies LinkTarget[];
