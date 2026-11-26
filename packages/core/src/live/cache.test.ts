import { create } from "@bufbuild/protobuf";
import { QueryClient } from "@tanstack/query-core";
import { live } from "@tokuly/api";
import { describe, expect, it, vi } from "vitest";
import { LiveChats, patchStream, storePoll } from "./cache";
import { liveKeys } from "./keys";

/** A chat client whose reads answer when the test says so. */
function slowChat() {
  const answers: ((messages: live.ChatMessage[]) => void)[] = [];
  const client = {
    listChatMessages: () =>
      new Promise((resolve) => answers.push((messages) => resolve({ messages }))),
  };
  return { chats: new LiveChats(() => client as never), answers };
}

const line = (id: string, offsetMs: number, userId = "u1") =>
  create(live.ChatMessageSchema, {
    id,
    streamId: "s1",
    offsetMs: BigInt(offsetMs),
    text: id,
    author: { user: { id: userId } },
  });

const ids = (messages: live.ChatMessage[] | undefined) => messages?.map((message) => message.id);

describe("reading a stream's chat again", () => {
  it("keeps what was pushed while it was read, and keeps out what was deleted meanwhile", async () => {
    const { chats, answers } = slowChat();
    const queryClient = new QueryClient();
    const key = liveKeys.chat("s1");
    queryClient.setQueryData(key, [line("old", 1), line("doomed", 2)]);
    const reading = queryClient.fetchQuery({ queryKey: key, queryFn: () => chats.read("s1") });
    await vi.waitFor(() => expect(answers).toHaveLength(1));

    // While the read is on its way: a new line, and a deletion.
    chats.apply(queryClient, "s1", { case: "chatMessage", value: line("pushed", 3) });
    chats.apply(queryClient, "s1", {
      case: "chatMessageDeleted",
      value: create(live.ChatMessageDeletedSchema, { messageId: "doomed" }),
    });
    // The answer was made before either.
    answers[0]?.([line("old", 1), line("doomed", 2)]);
    expect(ids(await reading)).toEqual(["old", "pushed"]);
    expect(ids(queryClient.getQueryData(key))).toEqual(["old", "pushed"]);
  });

  it("drops a line deleted while the app was not listening", async () => {
    const { chats, answers } = slowChat();
    const reading = chats.read("s2");
    await vi.waitFor(() => expect(answers).toHaveLength(1));
    answers[0]?.([line("still-there", 2)]);
    expect(ids(await reading)).toEqual(["still-there"]);
  });
});

describe("pushed events", () => {
  const stream = (id = "s1") =>
    create(live.StreamSchema, { summary: { id, shortId: `${id}-short`, viewerCount: 1n } });

  it("keep out the lines of people the viewer blocked", () => {
    const { chats } = slowChat();
    const queryClient = new QueryClient();
    queryClient.setQueryData(liveKeys.blocked, new Set(["bad"]));
    queryClient.setQueryData(liveKeys.chat("s1"), [] as live.ChatMessage[]);
    queryClient.setQueryData(liveKeys.stream("s1-short"), stream());
    chats.apply(queryClient, "s1", { case: "chatMessage", value: line("a", 1, "bad") });
    chats.apply(queryClient, "s1", { case: "chatMessage", value: line("b", 2, "good") });
    chats.apply(queryClient, "s1", {
      case: "chatPinChanged",
      value: create(live.ChatPinChangedSchema, { message: line("a", 1, "bad") }),
    });
    expect(ids(queryClient.getQueryData(liveKeys.chat("s1")))).toEqual(["b"]);
    expect(queryClient.getQueryData<live.Stream>(liveKeys.stream("s1-short"))?.pinnedMessage).toBe(
      undefined,
    );
  });

  it("change the stream under whichever ID it was opened with", () => {
    const { chats } = slowChat();
    const queryClient = new QueryClient();
    queryClient.setQueryData(liveKeys.stream("s1-short"), stream());
    chats.apply(queryClient, "s1", {
      case: "stats",
      value: create(live.StreamStatsSchema, { viewerCount: 42n, likeCount: 7n }),
    });
    const summary = queryClient.getQueryData<live.Stream>(liveKeys.stream("s1-short"))?.summary;
    expect(summary?.viewerCount).toBe(42n);
    expect(summary?.likeCount).toBe(7n);
  });

  it("put a gift on top of the highlights and take a deleted one away", () => {
    const { chats } = slowChat();
    const queryClient = new QueryClient();
    queryClient.setQueryData(liveKeys.stream("s1"), stream());
    const gift = create(live.ChatMessageSchema, {
      id: "g",
      streamId: "s1",
      kind: live.ChatMessageKind.GIFT,
      gift: { coins: 500n, highlightedUntil: { seconds: 99n, nanos: 0 } },
    });
    chats.apply(queryClient, "s1", { case: "chatMessage", value: gift });
    const read = () => queryClient.getQueryData<live.Stream>(liveKeys.stream("s1"));
    expect(ids(read()?.highlightedGifts)).toEqual(["g"]);
    chats.apply(queryClient, "s1", {
      case: "chatMessageDeleted",
      value: create(live.ChatMessageDeletedSchema, { messageId: "g" }),
    });
    expect(ids(read()?.highlightedGifts)).toEqual([]);
  });

  it("keep the viewer's vote on a pushed poll", () => {
    const { chats } = slowChat();
    const queryClient = new QueryClient();
    queryClient.setQueryData(liveKeys.stream("s1"), stream());
    storePoll(queryClient, create(live.PollSchema, { id: "p", streamId: "s1", viewerChoice: 2 }));
    chats.apply(queryClient, "s1", {
      case: "pollUpdated",
      value: create(live.PollSchema, { id: "p", streamId: "s1", totalVotes: 3n }),
    });
    const poll = queryClient.getQueryData<live.Stream>(liveKeys.stream("s1"))?.activePoll;
    expect(poll?.viewerChoice).toBe(2);
    expect(poll?.totalVotes).toBe(3n);
  });

  it("leave other streams alone", () => {
    const queryClient = new QueryClient();
    queryClient.setQueryData(liveKeys.stream("other"), stream("s9"));
    patchStream(queryClient, "s1", () => {
      throw new Error("patched the wrong stream");
    });
  });
});
