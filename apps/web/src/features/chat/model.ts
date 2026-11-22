import { chat, type common, toMillis } from "@tokuly/api";
import type { ListFilter } from "./api";

/** Pure reading helpers for the chat messages of the API. No state, no I/O. */

export function isGroup(conversation: Pick<chat.Conversation, "kind">): boolean {
  return conversation.kind === chat.ConversationKind.GROUP;
}

/** The other person of a direct conversation (undefined for groups). */
export function peerOf(conversation: chat.Conversation, myId: string): common.UserSummary | undefined {
  if (isGroup(conversation)) return undefined;
  return conversation.members.find((member) => member.id !== myId) ?? conversation.members[0];
}

/** Whether a mute is in force right now (a timed mute ends by itself). */
export function isMuted(viewer: chat.ViewerState | undefined, nowMs = Date.now()): boolean {
  if (!viewer?.muted) return false;
  return !viewer.mutedUntil || toMillis(viewer.mutedUntil) > nowMs;
}

/**
 * The list a conversation shows up in for its viewer, as the server lists
 * them. `null`: in none (hidden, declined, or a direct conversation nobody
 * has written in yet).
 */
export function listOf(conversation: chat.Conversation): ListFilter | null {
  const viewer = conversation.viewer;
  if (!viewer || viewer.hidden) return null;
  if (!isGroup(conversation) && conversation.lastSeq <= 0n) return null;
  switch (viewer.requestState) {
    case chat.RequestState.PENDING:
      return "requests";
    case chat.RequestState.DECLINED:
      return null;
    default:
      return viewer.archived ? "archived" : "inbox";
  }
}

/** List order: pinned conversations first, then the latest activity. */
export function compareConversations(a: chat.Conversation, b: chat.Conversation): number {
  const pinned = Number(b.viewer?.pinned ?? false) - Number(a.viewer?.pinned ?? false);
  if (pinned !== 0) return pinned;
  const activity = toMillis(b.lastActivityAt) - toMillis(a.lastActivityAt);
  if (activity !== 0) return activity;
  return a.id < b.id ? 1 : a.id > b.id ? -1 : 0;
}

/** The later of two sequence numbers. */
export function laterSeq(a: bigint, b: bigint): bigint {
  if (a > b) return a;
  return b;
}

/** Whether the viewer may write here at all. */
export function canWrite(conversation: chat.Conversation): boolean {
  return !conversation.readOnly && conversation.viewer?.requestState !== chat.RequestState.DECLINED;
}

/** Owner or admin of a group. */
export function manages(role: chat.MemberRole | undefined): boolean {
  return role === chat.MemberRole.OWNER || role === chat.MemberRole.ADMIN;
}

/** Whether a message adds to other members' unread counts (as the server counts). */
export function countsAsUnread(message: chat.Message): boolean {
  return Boolean(message.sender) && message.kind !== chat.MessageKind.SYSTEM && !message.deleted;
}

/** Whether a message mentions `userId`, by handle or through `@all`. */
export function mentionsUser(message: chat.Message, userId: string): boolean {
  return message.mentionsEveryone || message.mentions.some((mention) => mention.user?.id === userId);
}

/** Kinds a person wrote themselves (as opposed to system, call and POV request entries). */
export function isUserContent(message: chat.Message): boolean {
  switch (message.kind) {
    case chat.MessageKind.TEXT:
    case chat.MessageKind.STAMP:
    case chat.MessageKind.MEDIA:
    case chat.MessageKind.LOCATION:
    case chat.MessageKind.HELLO:
      return true;
    default:
      return false;
  }
}

/** What can be done to a message, by the rules of the API. */
export interface MessageAbilities {
  reply: boolean;
  react: boolean;
  copy: boolean;
  edit: boolean;
  forward: boolean;
  pin: boolean;
  delete: boolean;
  translate: boolean;
}

export function abilitiesOf(
  message: chat.Message,
  context: { myId: string; role: chat.MemberRole | undefined; writable: boolean; accepted: boolean; editWindowSeconds: number; nowMs?: number },
): MessageAbilities {
  const mine = message.sender?.id === context.myId;
  const content = isUserContent(message) && !message.deleted;
  const active = context.writable && context.accepted;
  const isPovReply = Boolean(message.povReply);
  const editable =
    mine &&
    active &&
    content &&
    !message.forwarded &&
    !isPovReply &&
    (message.kind === chat.MessageKind.TEXT || message.kind === chat.MessageKind.MEDIA) &&
    (context.nowMs ?? Date.now()) - toMillis(message.createdAt) < context.editWindowSeconds * 1000;
  return {
    reply: content && context.writable,
    react: content && active,
    copy: content && message.text.length > 0,
    edit: editable,
    forward: content && message.kind !== chat.MessageKind.HELLO && !isPovReply,
    pin: content && active,
    delete: content && (mine || manages(context.role)),
    translate: content && message.text.trim().length > 0,
  };
}

/** A participant of a call in a given state. */
export function participantState(call: chat.Call, userId: string): chat.CallParticipantState {
  return call.participants.find((participant) => participant.user?.id === userId)?.state ?? chat.CallParticipantState.UNSPECIFIED;
}

/** Whether this call should ring for `userId` right now. */
export function ringsFor(call: chat.Call, userId: string): boolean {
  return call.state === chat.CallState.RINGING && call.startedBy?.id !== userId && participantState(call, userId) === chat.CallParticipantState.RINGING;
}

/** The people connected to a call. */
export function joinedParticipants(call: chat.Call): common.UserSummary[] {
  return call.participants.flatMap((participant) => (participant.state === chat.CallParticipantState.JOINED && participant.user ? [participant.user] : []));
}
