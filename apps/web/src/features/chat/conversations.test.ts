import { QueryClient } from "@tanstack/react-query";
import { chat, create, fromDate } from "@tokuly/api";
import type { ListData } from "@tokuly/core/chat";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The server, as far as the lists need it: every answer is handed out by the test.
const answers: ((conversations: chat.Conversation[]) => void)[] = [];
const held = <T>(answer: (conversations: chat.Conversation[]) => T) =>
  new Promise<T>((resolve) => answers.push((conversations) => resolve(answer(conversations))));
vi.mock("./api", async (original) => ({
  ...(await original<typeof import("./api")>()),
  conversationClient: () => ({
    listConversations: () =>
      held((conversations) => create(chat.ListConversationsResponseSchema, { conversations })),
    getConversation: () =>
      held(([conversation]) => create(chat.GetConversationResponseSchema, { conversation })),
  }),
}));

const { chatKeys } = await import("./api");
const { conversationListQuery, fetchConversation } = await import("./conversations");
const { applyChatEvent } = await import("./events");

const conversation = (id: string) =>
  create(chat.ConversationSchema, {
    id,
    kind: chat.ConversationKind.GROUP,
    lastSeq: 2n,
    lastActivityAt: fromDate(new Date(2026, 10, 3)),
    viewer: { requestState: chat.RequestState.ACCEPTED, readSeq: 2n },
  });
const removed = (conversationId: string): chat.ChatEvent["event"] => ({
  case: "conversationRemoved",
  value: create(chat.ConversationRemovedSchema, {
    conversationId,
    reason: chat.ConversationRemovalReason.REMOVED,
  }),
});
const context = { myId: "me", readingId: null };
const inbox = (queryClient: QueryClient) =>
  queryClient
    .getQueryData<ListData>(chatKeys.list("inbox"))
    ?.pages.flatMap((page) => page.conversations.map((item) => item.id));
const settled = () => new Promise((resolve) => setTimeout(resolve, 0));

let queryClient: QueryClient;
beforeEach(() => {
  answers.length = 0;
  queryClient = new QueryClient();
});

describe("a conversation the viewer was removed from", () => {
  it("does not come back with a list read before the removal", async () => {
    const loading = queryClient.fetchInfiniteQuery(conversationListQuery(queryClient, "inbox"));
    await settled();
    answers[0]?.([conversation("c1"), conversation("c2")]);
    await loading;
    expect(inbox(queryClient)).toEqual(["c1", "c2"]);

    // Read again (a Welcome)…
    const again = queryClient.fetchInfiniteQuery({
      ...conversationListQuery(queryClient, "inbox"),
      staleTime: 0,
    });
    await settled();
    expect(answers).toHaveLength(2);
    applyChatEvent(queryClient, removed("c1"), context);
    expect(inbox(queryClient)).toEqual(["c2"]);
    // …answered from before the removal.
    answers[1]?.([conversation("c1"), conversation("c2")]);
    await again;
    expect(inbox(queryClient)).toEqual(["c2"]);
  });

  it("does not come back with the conversation fetched on its own before", async () => {
    queryClient.setQueryData<ListData>(chatKeys.list("inbox"), {
      pages: [{ conversations: [conversation("c1"), conversation("c2")], nextCursor: "" }],
      pageParams: [""],
    });
    const fetching = fetchConversation(queryClient, "c1");
    await settled();
    expect(answers).toHaveLength(1);
    applyChatEvent(queryClient, removed("c1"), context);
    answers[0]?.([conversation("c1")]);
    await fetching;
    expect(inbox(queryClient)).toEqual(["c2"]);
  });
});
