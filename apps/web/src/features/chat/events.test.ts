import { QueryClient } from "@tanstack/react-query";
import { chat, create, fromDate, type MessageInitShape } from "@tokuly/api";
import { beforeEach, describe, expect, it } from "vitest";
import { chatKeys } from "./api";
import { type ListPage, NO_UNREAD, readConversation, type Unread } from "./conversations";
import { applyChatEvent } from "./events";
import { useLive } from "./live";
import { make } from "./test-support";
import type { Timeline } from "./timeline";

const ME = "me";
const user = (id: string) => ({ id, handle: id, displayName: id });

function conversation(id: string, viewer: MessageInitShape<typeof chat.ViewerStateSchema> = {}, extra: MessageInitShape<typeof chat.ConversationSchema> = {}): chat.Conversation {
  return make(
    chat.ConversationSchema,
    {
      id,
      kind: chat.ConversationKind.DIRECT,
      members: [user(ME), user("aoi")],
      lastSeq: 2n,
      lastActivityAt: fromDate(new Date(2026, 10, 3, 10)),
      viewer: make(chat.ViewerStateSchema, { requestState: chat.RequestState.ACCEPTED, readSeq: 2n }, viewer),
    },
    extra,
  );
}

function message(seq: number, sender: string, extra: MessageInitShape<typeof chat.MessageSchema> = {}): chat.Message {
  return make(
    chat.MessageSchema,
    { id: `m${seq}`, conversationId: "c1", seq: BigInt(seq), sender: user(sender), kind: chat.MessageKind.TEXT, text: `text ${seq}`, createdAt: fromDate(new Date(2026, 10, 3, 10, seq)) },
    extra,
  );
}

/** A client holding an inbox with the given conversations and a timeline of "c1" at the live end. */
function client(conversations: chat.Conversation[], unread: Unread = NO_UNREAD): QueryClient {
  const queryClient = new QueryClient();
  queryClient.setQueryData(chatKeys.list("inbox"), { pages: [{ conversations, nextCursor: "" } satisfies ListPage], pageParams: [""] });
  queryClient.setQueryData(chatKeys.unread, unread);
  queryClient.setQueryData<Timeline>(chatKeys.timeline("c1"), { messages: [message(1, "aoi"), message(2, ME)], olderCursor: "", hasOlder: false, newerCursor: "", hasNewer: false });
  return queryClient;
}

const created = (item: chat.Message): chat.ChatEvent["event"] => ({ case: "messageCreated", value: create(chat.MessageCreatedSchema, { message: item }) });
const context = { myId: ME, readingId: null };
const unreadOf = (queryClient: QueryClient) => queryClient.getQueryData<Unread>(chatKeys.unread);
const timelineOf = (queryClient: QueryClient) => queryClient.getQueryData<Timeline>(chatKeys.timeline("c1"))?.messages.map((item) => Number(item.seq));

beforeEach(() => useLive.getState().reset());

describe("chat events", () => {
  it("adds a message from someone else to the conversation, its preview and the unread counts", () => {
    const queryClient = client([conversation("c1")]);
    applyChatEvent(queryClient, created(message(3, "aoi", { text: "おはよ" })), context);

    expect(timelineOf(queryClient)).toEqual([1, 2, 3]);
    const held = readConversation(queryClient, "c1");
    expect(held?.lastMessage?.text).toBe("おはよ");
    expect(held?.lastSeq).toBe(3n);
    expect(held?.viewer?.unreadCount).toBe(1);
    expect(unreadOf(queryClient)).toMatchObject({ badge: 1, unreadConversations: 1 });

    // A second one: one more message, still one conversation with unread messages.
    applyChatEvent(queryClient, created(message(4, "aoi")), context);
    expect(unreadOf(queryClient)).toMatchObject({ badge: 2, unreadConversations: 1 });
  });

  it("does not count a message twice when it is announced twice", () => {
    const queryClient = client([conversation("c1")]);
    applyChatEvent(queryClient, created(message(3, "aoi")), context);
    applyChatEvent(queryClient, created(message(3, "aoi")), context);
    expect(timelineOf(queryClient)).toEqual([1, 2, 3]);
    expect(unreadOf(queryClient)?.badge).toBe(1);
  });

  it("counts nothing as unread in the conversation that is being read", () => {
    const queryClient = client([conversation("c1")]);
    applyChatEvent(queryClient, created(message(3, "aoi")), { myId: ME, readingId: "c1" });
    expect(timelineOf(queryClient)).toEqual([1, 2, 3]);
    expect(readConversation(queryClient, "c1")?.viewer?.unreadCount).toBe(0);
    expect(unreadOf(queryClient)?.badge).toBe(0);
  });

  it("keeps a muted conversation out of the badge but still counts its unread messages", () => {
    const queryClient = client([conversation("c1", { muted: true })]);
    applyChatEvent(queryClient, created(message(3, "aoi")), context);
    expect(readConversation(queryClient, "c1")?.viewer?.unreadCount).toBe(1);
    expect(unreadOf(queryClient)).toMatchObject({ badge: 0, unreadConversations: 1 });
  });

  it("notes a mention of the viewer", () => {
    const queryClient = client([conversation("c1")]);
    applyChatEvent(queryClient, created(message(3, "aoi", { mentions: [{ handle: ME, user: user(ME) }] })), context);
    expect(readConversation(queryClient, "c1")?.viewer?.unreadMentionCount).toBe(1);
  });

  it("treats the viewer's own message (sent from another device) as read, not unread", () => {
    const queryClient = client([conversation("c1", { unreadCount: 0 })]);
    applyChatEvent(queryClient, created(message(3, ME)), context);
    const viewer = readConversation(queryClient, "c1")?.viewer;
    expect(viewer?.unreadCount).toBe(0);
    expect(viewer?.readSeq).toBe(3n);
    expect(unreadOf(queryClient)?.badge).toBe(0);
  });

  it("does not count system notices as unread", () => {
    const queryClient = client([conversation("c1")]);
    applyChatEvent(queryClient, created(message(3, "aoi", { kind: chat.MessageKind.SYSTEM, sender: undefined })), context);
    expect(readConversation(queryClient, "c1")?.viewer?.unreadCount).toBe(0);
  });

  it("replaces an edited or deleted message, also in the list preview", () => {
    const queryClient = client([conversation("c1", {}, { lastMessage: message(2, ME) })]);
    const event: chat.ChatEvent["event"] = { case: "messageUpdated", value: create(chat.MessageUpdatedSchema, { message: message(2, ME, { text: "", deleted: true }), reason: chat.MessageUpdateReason.DELETED }) };
    applyChatEvent(queryClient, event, context);
    expect(queryClient.getQueryData<Timeline>(chatKeys.timeline("c1"))?.messages[1]?.deleted).toBe(true);
    expect(readConversation(queryClient, "c1")?.lastMessage?.deleted).toBe(true);
  });

  it("tracks reactions, telling the viewer's own apart", () => {
    const queryClient = client([conversation("c1")]);
    const change = (userId: string, added: boolean, count: number): chat.ChatEvent["event"] => ({
      case: "reactionChanged",
      value: create(chat.ReactionChangedSchema, { conversationId: "c1", messageId: "m1", emoji: "❤️", userId, added, count }),
    });
    applyChatEvent(queryClient, change("aoi", true, 1), context);
    applyChatEvent(queryClient, change(ME, true, 2), context);
    const reactions = () => queryClient.getQueryData<Timeline>(chatKeys.timeline("c1"))?.messages[0]?.reactions.map((reaction) => [reaction.count, reaction.reactedByMe]);
    expect(reactions()).toEqual([[2, true]]);
    applyChatEvent(queryClient, change("aoi", false, 1), context);
    expect(reactions()).toEqual([[1, true]]);
  });

  it("moves read cursors forward only, and clears the viewer's counts when they read elsewhere", () => {
    const queryClient = client([conversation("c1", { unreadCount: 2, readSeq: 2n }, { lastSeq: 4n })]);
    const read = (userId: string, readSeq: bigint): chat.ChatEvent["event"] => ({ case: "readReceipt", value: create(chat.ReadReceiptChangedSchema, { conversationId: "c1", userId, readSeq }) });

    applyChatEvent(queryClient, read("aoi", 2n), context);
    applyChatEvent(queryClient, read("aoi", 1n), context);
    expect(useLive.getState().cursors.c1?.aoi).toBe(2n);

    applyChatEvent(queryClient, read(ME, 4n), context);
    expect(readConversation(queryClient, "c1")?.viewer).toMatchObject({ unreadCount: 0, readSeq: 4n });
  });

  it("keeps the viewer's own state when only the shared part of a conversation changed", () => {
    const queryClient = client([conversation("c1", { unreadCount: 3, pinned: true }, { kind: chat.ConversationKind.GROUP, name: "old", lastMessage: message(2, ME) })]);
    const renamed = create(chat.ConversationSchema, { id: "c1", kind: chat.ConversationKind.GROUP, name: "new", lastSeq: 2n, lastActivityAt: fromDate(new Date(2026, 10, 3, 10)) });
    applyChatEvent(queryClient, { case: "conversationUpdated", value: create(chat.ConversationUpdatedSchema, { conversation: renamed }) }, context);
    const held = readConversation(queryClient, "c1");
    expect(held?.name).toBe("new");
    expect(held?.viewer).toMatchObject({ unreadCount: 3, pinned: true });
    expect(held?.lastMessage?.id).toBe("m2");
  });

  it("forgets a conversation that left the viewer's lists", () => {
    const queryClient = client([conversation("c1"), conversation("c2")]);
    applyChatEvent(queryClient, { case: "conversationRemoved", value: create(chat.ConversationRemovedSchema, { conversationId: "c1", reason: chat.ConversationRemovalReason.LEFT }) }, context);
    expect(readConversation(queryClient, "c1")).toBeUndefined();
    expect(readConversation(queryClient, "c2")).toBeDefined();
    expect(queryClient.getQueryData(chatKeys.timeline("c1"))).toBeUndefined();
  });

  it("shows who is typing and who is here", () => {
    const queryClient = client([conversation("c1")]);
    applyChatEvent(queryClient, { case: "typing", value: create(chat.TypingChangedSchema, { conversationId: "c1", userId: "aoi", typing: true }) }, context);
    expect(Object.keys(useLive.getState().typing.c1 ?? {})).toEqual(["aoi"]);
    applyChatEvent(queryClient, { case: "typing", value: create(chat.TypingChangedSchema, { conversationId: "c1", userId: "aoi", typing: false }) }, context);
    expect(useLive.getState().typing.c1).toEqual({});
    applyChatEvent(queryClient, { case: "presence", value: create(chat.PresenceChangedSchema, { conversationId: "c1", presentUserIds: ["aoi", ME] }) }, context);
    expect(useLive.getState().present.c1).toEqual(["aoi", ME]);
  });

  it("follows a call from ringing to its end, and never brings an ended call back", () => {
    const queryClient = client([conversation("c1")]);
    const call = (state: chat.CallState): chat.ChatEvent["event"] => ({
      case: "call",
      value: create(chat.CallChangedSchema, { call: { id: "call1", conversationId: "c1", state, startedBy: user("aoi"), participants: [{ user: user(ME), state: chat.CallParticipantState.RINGING }] } }),
    });
    applyChatEvent(queryClient, call(chat.CallState.RINGING), context);
    expect(useLive.getState().calls.call1?.state).toBe(chat.CallState.RINGING);
    expect(readConversation(queryClient, "c1")?.activeCallId).toBe("call1");

    applyChatEvent(queryClient, call(chat.CallState.ENDED), context);
    expect(useLive.getState().calls.call1).toBeUndefined();
    expect(readConversation(queryClient, "c1")?.activeCallId).toBe("");

    // A late "ringing" (an answer that was on its way) must not ring again.
    applyChatEvent(queryClient, call(chat.CallState.RINGING), context);
    expect(useLive.getState().calls.call1).toBeUndefined();
  });
});

describe("live state", () => {
  it("does not let an older answer undo a newer event", () => {
    const live = useLive.getState();
    const request = create(chat.PovRequestSchema, { id: "p1", conversationId: "c1", state: chat.PovState.OPEN });
    // The event arrives now; the answer to a question asked a second earlier knows nothing of it.
    live.setOpenPov("c1", request);
    live.setOpenPov("c1", undefined, Date.now() - 1000);
    expect(useLive.getState().openPov.c1?.id).toBe("p1");
    live.setPresent("c1", ["aoi"]);
    live.setPresent("c1", [], Date.now() - 1000);
    expect(useLive.getState().present.c1).toEqual(["aoi"]);
    // A newer one still wins.
    live.setOpenPov("c1", { ...request, state: chat.PovState.DECLINED });
    expect(useLive.getState().openPov.c1).toBeUndefined();
  });
});
