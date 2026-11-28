import { QueryClient } from "@tanstack/query-core";
import { chat, create, fromDate } from "@tokuly/api";
import { afterEach, describe, expect, it } from "vitest";
import { journal } from "../query";
import { applyChatEvent, type ChatSync } from "./events";
import { chatKeys } from "./keys";
import { readTimeline, storeAnswer, toggleReaction } from "./message-cache";
import { MessagePager, type Position } from "./pager";
import { type Timeline, timelineOf } from "./timeline";
import { UnreadTotals } from "./totals";

/**
 * Pages of a conversation's messages read before a message was edited,
 * unsent or reacted to, and answered after the push: the push stays.
 */

const message = (seq: number, text = `text ${seq}`, extra: { deleted?: boolean } = {}) =>
  create(chat.MessageSchema, {
    id: `m${seq}`,
    conversationId: "c1",
    seq: BigInt(seq),
    kind: chat.MessageKind.TEXT,
    sender: { id: "aoi", handle: "aoi" },
    text,
    createdAt: fromDate(new Date(2026, 10, 3, 10, seq)),
    ...extra,
  });

/** The server, answering each page when the test says, with what it held when asked. */
function server() {
  const asked: { position: Position; answer: (messages: chat.Message[]) => void }[] = [];
  const pager = new MessagePager(
    (_conversationId, position) =>
      new Promise((resolve) =>
        asked.push({
          position,
          answer: (messages) =>
            resolve(
              create(chat.ListMessagesResponseSchema, {
                messages,
                olderCursor: "older",
                hasOlder: position.case === "before" ? false : messages[0]?.seq !== 1n,
              }),
            ),
        }),
      ),
  );
  return { asked, pager };
}

const totals = new UnreadTotals();
afterEach(() => totals.dispose());
const sync = (pager: MessagePager): ChatSync => ({
  totals,
  pager,
  fetchConversation: () => Promise.resolve(),
});
const context = { myId: "me", readingId: "c1" };
const updated = (item: chat.Message, reason: chat.MessageUpdateReason) =>
  ({
    case: "messageUpdated",
    value: create(chat.MessageUpdatedSchema, { message: item, reason }),
  }) as const;
const texts = (queryClient: QueryClient) =>
  readTimeline(queryClient, "c1")?.messages.map((item) => (item.deleted ? "×" : item.text));
const settled = () => new Promise((resolve) => setTimeout(resolve, 0));

function holding(messages: chat.Message[]): QueryClient {
  const queryClient = new QueryClient();
  queryClient.setQueryData<Timeline>(
    chatKeys.timeline("c1"),
    timelineOf({
      messages,
      olderCursor: "older",
      hasOlder: true,
      newerCursor: "",
      hasNewer: false,
    }),
  );
  return queryClient;
}

describe("a page read before a message changed", () => {
  it("does not bring back the old text of a message edited while the first page loaded", async () => {
    const queryClient = new QueryClient();
    const { asked, pager } = server();
    const opening = queryClient.fetchQuery({
      queryKey: chatKeys.timeline("c1"),
      queryFn: ({ signal }) => pager.first("c1", undefined, signal),
      persister: journal,
    });
    await settled();
    applyChatEvent(
      queryClient,
      updated(message(2, "やっぱ11時で！"), chat.MessageUpdateReason.EDITED),
      context,
      sync(pager),
    );
    asked[0]?.answer([message(1), message(2, "10時でどう？")]);
    await opening;
    expect(texts(queryClient)).toEqual(["text 1", "やっぱ11時で！"]);
  });

  it("does not bring back a message unsent while an older page was read", async () => {
    const queryClient = holding([message(3), message(4)]);
    const { asked, pager } = server();
    const loading = pager.loadOlder(queryClient, "c1");
    await settled();
    applyChatEvent(
      queryClient,
      updated(message(2, "", { deleted: true }), chat.MessageUpdateReason.DELETED),
      context,
      sync(pager),
    );
    asked[0]?.answer([message(1), message(2, "まちがえた")]);
    await loading;
    expect(texts(queryClient)).toEqual(["text 1", "×", "text 3", "text 4"]);
  });

  it("keeps an edit pushed while the timeline jumped to the newest messages", async () => {
    const queryClient = holding([message(1)]);
    const { asked, pager } = server();
    const jumping = pager.showNewest(queryClient, "c1");
    await settled();
    applyChatEvent(
      queryClient,
      updated(message(9, "なおした"), chat.MessageUpdateReason.EDITED),
      context,
      sync(pager),
    );
    asked[0]?.answer([message(8), message(9, "まちがい")]);
    await jumping;
    expect(texts(queryClient)).toEqual(["text 8", "なおした"]);
  });

  it("adds a message pushed while the first page loaded, unless messages before it are missing", async () => {
    const queryClient = new QueryClient();
    const { asked, pager } = server();
    const opening = queryClient.fetchQuery({
      queryKey: chatKeys.timeline("c1"),
      queryFn: ({ signal }) => pager.first("c1", undefined, signal),
      persister: journal,
    });
    await settled();
    // The conversation is not held: nothing else to update.
    const created = (item: chat.Message) =>
      ({
        case: "messageCreated",
        value: create(chat.MessageCreatedSchema, { message: item }),
      }) as const;
    queryClient.setQueryData(
      chatKeys.conversation("c1"),
      create(chat.ConversationSchema, { id: "c1" }),
    );
    applyChatEvent(queryClient, created(message(3)), context, sync(pager));
    applyChatEvent(queryClient, created(message(5)), context, sync(pager));
    asked[0]?.answer([message(1), message(2)]);
    await opening;
    // 4 is missing: 5 comes with the page that fetches it.
    expect(texts(queryClient)).toEqual(["text 1", "text 2", "text 3"]);
  });
});

describe("an answer older than a push", () => {
  it("keeps an edit made after the one being saved", async () => {
    const queryClient = holding([message(1, "はじめ")]);
    let answer: (item: chat.Message) => void = () => {};
    const saving = storeAnswer(queryClient, "c1", async () => ({
      message: await new Promise<chat.Message>((resolve) => (answer = resolve)),
    }));
    applyChatEvent(
      queryClient,
      updated(message(1, "にかいめ"), chat.MessageUpdateReason.EDITED),
      context,
      sync(server().pager),
    );
    answer(message(1, "いっかいめ"));
    await saving;
    expect(texts(queryClient)).toEqual(["にかいめ"]);
  });

  it("keeps a reaction count pushed while the viewer's own reaction was saved", async () => {
    const queryClient = holding([message(1)]);
    let answer: (reactions: chat.ReactionCount[]) => void = () => {};
    const client = () =>
      ({
        addReaction: () =>
          new Promise((resolve) => (answer = (reactions) => resolve({ reactions }))),
      }) as never;
    const reacting = toggleReaction(queryClient, client, message(1), "❤️");
    // Someone else reacts too; their count arrives first.
    applyChatEvent(
      queryClient,
      {
        case: "reactionChanged",
        value: create(chat.ReactionChangedSchema, {
          conversationId: "c1",
          messageId: "m1",
          userId: "ren",
          emoji: "❤️",
          added: true,
          count: 2,
        }),
      },
      context,
      sync(server().pager),
    );
    answer([create(chat.ReactionCountSchema, { emoji: "❤️", count: 1, reactedByMe: true })]);
    await reacting;
    expect(readTimeline(queryClient, "c1")?.messages[0]?.reactions).toMatchObject([
      { emoji: "❤️", count: 2, reactedByMe: true },
    ]);
  });
});
