/**
 * What the community feature shares with other features.
 *
 *   communityHref("neko")                       // "/g/neko": a community's front page
 *   threadHref("neko", thread.shortId)          // "/g/neko/t/<thread>"
 *   eventHref("neko", event.id)                 // "/g/neko/events/<event>"
 *   inviteHref(code)                            // "/join/<code>": what an invite link opens
 *   appPathOf("/c/neko/t/bXou6FpLkfK")          // the same page under this app's address, or `undefined`
 *
 * The community domain publishes its pages under `/c/<handle>/…`; in this
 * app `/c/<handle>` is a video channel, so a path from the API (a
 * notification, a search hit, a subject preview) goes through `appPathOf`
 * before it is opened.
 *
 *   <CommunityIcon community={community} size="sm" />   // its picture, or the emoji of its category
 */
export { appPathOf, communityHref, eventHref, inviteHref, threadHref } from "./model";
export { CommunityIcon } from "./parts";
