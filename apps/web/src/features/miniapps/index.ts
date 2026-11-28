/**
 * What the mini-app feature shares with other features (`docs/miniapps.md`).
 *
 * `<MiniAppLinkCard>` is how a link to a mini app shows in a chat message:
 * its icon and name and a button that opens it with the conversation it was
 * sent in, as far as the viewer may open it:
 *
 *   <MiniAppLinkCard link={readMiniAppLink(href, origin)} conversationId={id} fallback={<Preview />} />
 *
 * The page that runs a mini app is `/miniapps/<id>` (`./screen`), the staff
 * review `/admin/mini-apps`.
 */
export { MiniAppLinkCard } from "./link-card";
