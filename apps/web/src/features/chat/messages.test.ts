import { QueryClient } from "@tanstack/react-query";
import { chat, create, fromDate } from "@tokuly/api";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The server, as far as the timeline needs it: every page is answered by the test.
const answers: ((messages: chat.Message[]) => void)[] = [];
vi.mock("./api", async (original) => ({
  ...(await original<typeof import("./api")>()),
  messageClient: () => ({
    listMessages: () =>
      new Promise((resolve) =>
        answers.push((messages) =>
          resolve(
            create(chat.ListMessagesResponseSchema, {
              messages,
              olderCursor: "older",
              hasOlder: messages[0]?.seq !== 1n,
            }),
          ),
        ),
      ),
  }),
}));

const { applyChatEvent } = await import("./events");
const { loadOlder, readTimeline, timelineQuery } = await import("./messages");

const message = (seq: number, text: string, extra: { deleted?: boolean } = {}) =>
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
const updated = (item: chat.Message, reason: chat.MessageUpdateReason) =>
  ({
    case: "messageUpdated",
    value: create(chat.MessageUpdatedSchema, { message: item, reason }),
  }) as const;
const context = { myId: "me", readingId: "c1" };
const texts = (queryClient: QueryClient) =>
  readTimeline(queryClient, "c1")?.messages.map((item) => (item.deleted ? "×" : item.text));
const settled = () => new Promise((resolve) => setTimeout(resolve, 0));

let queryClient: QueryClient;
beforeEach(() => {
  answers.length = 0;
  queryClient = new QueryClient();
});

describe("a page read before a message changed", () => {
  it("does not bring back the text of a message edited while the conversation opened", async () => {
    const opening = queryClient.fetchQuery(timelineQuery("c1"));
    await settled();
    expect(answers).toHaveLength(1);
    applyChatEvent(
      queryClient,
      updated(message(2, "やっぱ11時で！"), chat.MessageUpdateReason.EDITED),
      context,
    );
    answers[0]?.([message(1, "明日なんじ？"), message(2, "10時でどう？")]);
    await opening;
    expect(texts(queryClient)).toEqual(["明日なんじ？", "やっぱ11時で！"]);
  });

  it("does not bring back a message unsent while older messages were loaded", async () => {
    const opening = queryClient.fetchQuery(timelineQuery("c1"));
    await settled();
    answers[0]?.([message(3, "あとで"), message(4, "またね")]);
    await opening;

    const loading = loadOlder(queryClient, "c1");
    await settled();
    expect(answers).toHaveLength(2);
    applyChatEvent(
      queryClient,
      updated(message(2, "", { deleted: true }), chat.MessageUpdateReason.DELETED),
      context,
    );
    answers[1]?.([message(1, "おはよ"), message(2, "まちがえた")]);
    await loading;
    expect(texts(queryClient)).toEqual(["おはよ", "×", "あとで", "またね"]);
  });
});
