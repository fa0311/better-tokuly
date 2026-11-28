/**
 * What the chat feature shares with other features.
 *
 * `openDirectChat(handleOrUserId)` goes to the direct conversation with a
 * person (creating it when the two never talked). Use it for "message"
 * buttons on profiles, map markers, friend lists:
 *
 *   <Button onClick={() => openDirectChat(user.id)}>メッセージ</Button>
 *
 * (A plain link does the same: `<Link to="/chat" search={{ user: user.handle }}>`.)
 *
 * `<ShareToChat>` is the "send to a friend" sheet for notes, videos, places
 * and any link: the person picks conversations, adds a note, and the link is
 * sent into each as a message that chat shows with its preview:
 *
 *   <ShareToChat open={sharing} onOpenChange={setSharing} share={{ subject: { type: common.SubjectType.NOTE, id: note.id } }} title={note.text} />
 *   <ShareToChat open={sharing} onOpenChange={setSharing} share={{ url: location.href }} />
 */

/** 「AI に自分のメッセージを使ってもいい」, for the privacy settings. */
export { AiMessageConsentSwitch } from "./ai-consent-switch";
/** Ticking friends from the caller's friend list (the mini-app friend picker). */
export { FriendPicker } from "./friend-picker";
export { openDirectChat } from "./navigation";
export { ShareToChat, type ShareToChatProps } from "./share-to-chat";
