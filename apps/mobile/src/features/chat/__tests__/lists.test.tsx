import { toBinary } from "@bufbuild/protobuf";
import { type QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react-native";
import { chat, create, fromDate } from "@tokuly/api";
import { applyChatEvent, chatKeys, type ListData } from "@tokuly/core/chat";
import type { ReactNode } from "react";
import { testQueryClient } from "#/testing/query";
import { chatSync, fetchConversation, useConversationList } from "../data";

const conversation = (id: string) =>
  create(chat.ConversationSchema, {
    id,
    kind: chat.ConversationKind.GROUP,
    lastSeq: 2n,
    lastActivityAt: fromDate(new Date(2026, 10, 3)),
    viewer: { requestState: chat.RequestState.ACCEPTED, readSeq: 2n },
  });

const proto = (body: Uint8Array<ArrayBuffer>) =>
  new Response(body, { status: 200, headers: { "content-type": "application/proto" } });

/**
 * The server: the conversations' answers are handed out by the test, with
 * what it held when asked; the unread totals answer at once.
 */
const answers: ((conversations: chat.Conversation[]) => void)[] = [];
async function mockServer(input: string): Promise<Response> {
  const method = new URL(input).pathname.split("/").pop();
  if (method === "GetUnreadCounts")
    return proto(
      toBinary(chat.GetUnreadCountsResponseSchema, create(chat.GetUnreadCountsResponseSchema)),
    );
  return new Promise((resolve) =>
    answers.push((conversations) =>
      resolve(
        proto(
          method === "ListConversations"
            ? toBinary(
                chat.ListConversationsResponseSchema,
                create(chat.ListConversationsResponseSchema, { conversations }),
              )
            : toBinary(
                chat.GetConversationResponseSchema,
                create(chat.GetConversationResponseSchema, { conversation: conversations[0] }),
              ),
        ),
      ),
    ),
  );
}
jest.mock("expo/fetch", () => ({ fetch: (input: string) => mockServer(input) }));

const removed = (conversationId: string): chat.ChatEvent["event"] => ({
  case: "conversationRemoved",
  value: create(chat.ConversationRemovedSchema, {
    conversationId,
    reason: chat.ConversationRemovalReason.LEFT,
  }),
});
const context = { myId: "me", readingId: null };

let queryClient: QueryClient;
afterEach(() => {
  queryClient.clear();
  chatSync.totals.dispose();
});
beforeEach(() => {
  answers.length = 0;
  queryClient = testQueryClient({ defaultOptions: { queries: { retry: false } } });
});

function render() {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return renderHook(() => useConversationList("inbox"), { wrapper });
}
const ids = (data: Pick<ListData, "pages"> | undefined) =>
  data?.pages.flatMap((page) => page.conversations.map((item) => item.id));
/** What the list screen draws: the inbox as the cache holds it. */
const inbox = () => ids(queryClient.getQueryData<ListData>(chatKeys.list("inbox")));

describe("a conversation the person left", () => {
  it("does not come back with the list read again before it was left", async () => {
    const { result } = await render();
    await waitFor(() => expect(answers).toHaveLength(1));
    await act(async () => answers[0]?.([conversation("c1"), conversation("c2")]));
    await waitFor(() => expect(ids(result.current.data)).toEqual(["c1", "c2"]));

    // Read again (a Welcome), and left on another device while the answer is on its way.
    let again: Promise<unknown> = Promise.resolve();
    await act(async () => {
      again = result.current.refetch();
    });
    expect(answers).toHaveLength(2);
    await act(async () => applyChatEvent(queryClient, removed("c1"), context, chatSync));
    await act(async () => {
      answers[1]?.([conversation("c1"), conversation("c2")]);
      await again;
    });
    expect(inbox()).toEqual(["c2"]);
  });

  it("does not come back with the conversation fetched on its own before", async () => {
    queryClient.setQueryData<ListData>(chatKeys.list("inbox"), {
      pages: [{ conversations: [conversation("c1"), conversation("c2")], nextCursor: "" }],
      pageParams: [""],
    });
    const fetching = fetchConversation(queryClient, "c1");
    await waitFor(() => expect(answers).toHaveLength(1));
    applyChatEvent(queryClient, removed("c1"), context, chatSync);
    answers[0]?.([conversation("c1")]);
    await fetching;
    expect(inbox()).toEqual(["c2"]);
  });
});
