import { toBinary } from "@bufbuild/protobuf";
import { type QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react-native";
import { chat, create, fromDate } from "@tokuly/api";
import { applyChatEvent, chatKeys, readTimeline } from "@tokuly/core/chat";
import type { ReactNode } from "react";
import { testQueryClient } from "#/testing/query";
import { chatSync, pager, useTimeline } from "../data";

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

/** The server: every page is answered by the test, with the messages it held when asked. */
const answers: ((messages: chat.Message[]) => void)[] = [];
function mockServer(): Promise<Response> {
  return new Promise((resolve) =>
    answers.push((messages) => {
      const page = create(chat.ListMessagesResponseSchema, {
        messages,
        olderCursor: "older",
        hasOlder: messages[0]?.seq !== 1n,
      });
      resolve(
        new Response(toBinary(chat.ListMessagesResponseSchema, page), {
          status: 200,
          headers: { "content-type": "application/proto" },
        }),
      );
    }),
  );
}
jest.mock("expo/fetch", () => ({ fetch: () => mockServer() }));

const updated = (item: chat.Message, reason: chat.MessageUpdateReason) =>
  ({
    case: "messageUpdated",
    value: create(chat.MessageUpdatedSchema, { message: item, reason }),
  }) as const;
const context = { myId: "me", readingId: "c1" };
const texts = () =>
  readTimeline(queryClient, "c1")?.messages.map((item) => (item.deleted ? "×" : item.text));
const asked = (count: number) =>
  act(async () => {
    while (answers.length < count) await new Promise((resolve) => setTimeout(resolve, 5));
  });

/** Waits for the first page to be stored. */
const opened = () =>
  act(async () => {
    await queryClient.getQueryCache().find({ queryKey: chatKeys.timeline("c1") })?.promise;
  });

let queryClient: QueryClient;
beforeEach(() => {
  answers.length = 0;
  queryClient = testQueryClient({ defaultOptions: { queries: { retry: false } } });
});
afterEach(() => {
  queryClient.clear();
  chatSync.totals.dispose();
});

function render() {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return renderHook(() => useTimeline("c1"), { wrapper });
}

describe("a page read before a message changed", () => {
  it("does not bring back the text of a message edited while the conversation opened", async () => {
    await render();
    await asked(1);
    await act(async () =>
      applyChatEvent(
        queryClient,
        updated(message(2, "やっぱ11時で！"), chat.MessageUpdateReason.EDITED),
        context,
        chatSync,
      ),
    );
    await act(async () => answers[0]?.([message(1, "明日なんじ？"), message(2, "10時でどう？")]));
    await opened();
    expect(texts()).toEqual(["明日なんじ？", "やっぱ11時で！"]);
  });

  it("does not bring back a message unsent while older messages were loaded", async () => {
    await render();
    await asked(1);
    await act(async () => answers[0]?.([message(3, "あとで"), message(4, "またね")]));
    await opened();

    let loading: Promise<void> = Promise.resolve();
    await act(async () => {
      loading = pager.loadOlder(queryClient, "c1");
    });
    await asked(2);
    await act(async () =>
      applyChatEvent(
        queryClient,
        updated(message(2, "", { deleted: true }), chat.MessageUpdateReason.DELETED),
        context,
        chatSync,
      ),
    );
    await act(async () => {
      answers[1]?.([message(1, "おはよ"), message(2, "まちがえた")]);
      await loading;
    });
    expect(texts()).toEqual(["おはよ", "×", "あとで", "またね"]);
  });
});
