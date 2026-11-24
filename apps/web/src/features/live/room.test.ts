import { create } from "@bufbuild/protobuf";
import { QueryClient } from "@tanstack/react-query";
import { live } from "@tokuly/api";
import { describe, expect, it, vi } from "vitest";

// The chat read answers when the test says so.
const answers: ((messages: live.ChatMessage[]) => void)[] = [];
vi.mock("./api", () => ({
  chatClient: () => ({
    listChatMessages: () =>
      new Promise((resolve) => answers.push((messages) => resolve({ messages }))),
  }),
  streamClient: () => ({}),
  liveKeys: {
    chat: (streamId: string) => ["live", "chat", streamId],
    blocked: ["live", "blocked"],
  },
  patchStream: () => {},
  refreshStream: () => {},
  useBlockedIds: () => {},
}));
vi.mock("#/lib/realtime", () => ({ realtime: () => ({}), deviceId: () => "device" }));

const { applyLiveEvent, readChat } = await import("./room");

const line = (id: string, offsetMs: number) =>
  create(live.ChatMessageSchema, { id, streamId: "s1", offsetMs: BigInt(offsetMs), text: id });

describe("reading a stream's chat again", () => {
  it("keeps what was pushed while it was read, and keeps out what was deleted meanwhile", async () => {
    const queryClient = new QueryClient();
    const key = ["live", "chat", "s1"];
    queryClient.setQueryData(key, [line("old", 1), line("doomed", 2)]);
    const reading = queryClient.fetchQuery({ queryKey: key, queryFn: () => readChat("s1") });
    await vi.waitFor(() => expect(answers).toHaveLength(1));

    // While the read is on its way: a new line, and a deletion.
    applyLiveEvent(queryClient, "s1", { case: "chatMessage", value: line("pushed", 3) });
    applyLiveEvent(queryClient, "s1", {
      case: "chatMessageDeleted",
      value: create(live.ChatMessageDeletedSchema, { messageId: "doomed" }),
    });
    // The answer was made before either.
    answers[0]?.([line("old", 1), line("doomed", 2)]);
    const messages = await reading;
    expect(messages.map((message) => message.id)).toEqual(["old", "pushed"]);
    expect(queryClient.getQueryData<live.ChatMessage[]>(key)?.map((message) => message.id)).toEqual(
      ["old", "pushed"],
    );
  });

  it("drops a line deleted while the page was not listening", async () => {
    const queryClient = new QueryClient();
    const key = ["live", "chat", "s2"];
    queryClient.setQueryData(key, [line("deleted-in-the-gap", 1)]);
    const reading = readChat("s2");
    await vi.waitFor(() => expect(answers).toHaveLength(2));
    answers[1]?.([line("still-there", 2)]);
    expect((await reading).map((message) => message.id)).toEqual(["still-there"]);
  });
});
