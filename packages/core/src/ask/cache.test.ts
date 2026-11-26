import { create } from "@bufbuild/protobuf";
import { QueryClient } from "@tanstack/query-core";
import { ask } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { applyAskEvent, dropFromInbox } from "./cache";
import { askKeys } from "./keys";

type Inbox = { pages: ask.ListInboxResponse[]; pageParams: string[] };

const question = (id: string) => create(ask.QuestionSchema, { id, text: id });
const inboxOf = (...ids: string[]): Inbox => ({
  pages: [create(ask.ListInboxResponseSchema, { questions: ids.map(question) })],
  pageParams: [""],
});
const idsIn = (queryClient: QueryClient, folder: ask.QuestionFolder) =>
  queryClient
    .getQueryData<Inbox>(askKeys.inbox(folder))
    ?.pages.flatMap((page) => page.questions.map((known) => known.id));

describe("the inbox in the cache", () => {
  it("a moved question leaves every loaded folder", () => {
    const queryClient = new QueryClient();
    queryClient.setQueryData(askKeys.inbox(ask.QuestionFolder.INBOX), inboxOf("q1", "q2"));
    queryClient.setQueryData(askKeys.inbox(ask.QuestionFolder.ARCHIVED), inboxOf("q1"));
    dropFromInbox(queryClient, "q1");
    expect(idsIn(queryClient, ask.QuestionFolder.INBOX)).toEqual(["q2"]);
    expect(idsIn(queryClient, ask.QuestionFolder.ARCHIVED)).toEqual([]);
  });

  it("a pushed question goes on top once, with the counts it carried", () => {
    const queryClient = new QueryClient();
    queryClient.setQueryData(askKeys.inbox(ask.QuestionFolder.INBOX), inboxOf("q1"));
    const event = {
      case: "questionReceived" as const,
      value: create(ask.QuestionReceivedSchema, {
        question: question("q2"),
        counts: { unread: 3 },
      }),
    };
    applyAskEvent(queryClient, event);
    applyAskEvent(queryClient, event);
    expect(idsIn(queryClient, ask.QuestionFolder.INBOX)).toEqual(["q2", "q1"]);
    expect(queryClient.getQueryData<ask.InboxCounts>(askKeys.counts)?.unread).toBe(3);
  });
});
