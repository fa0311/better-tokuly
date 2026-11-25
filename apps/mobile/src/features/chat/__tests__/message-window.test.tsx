import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { chat, create } from "@tokuly/api";
import { act, renderHook, waitFor } from "@testing-library/react-native";
import type { ReactNode } from "react";
import { useLive } from "../live";
import { useMessageWindow } from "../message-window";
import { useOutbox } from "../outbox";

/** A conversation of messages 1…60 on the server, paged 20 at a time; Aoi has read up to 55. */
const ALL = Array.from({ length: 60 }, (_, index) =>
  create(chat.MessageSchema, {
    id: `m${index + 1}`,
    conversationId: "c1",
    seq: BigInt(index + 1),
    kind: chat.MessageKind.TEXT,
    text: `message ${index + 1}`,
    sender: { id: index % 2 ? "me" : "aoi", handle: index % 2 ? "me" : "aoi" },
  }),
);
const asked: string[] = [];

function page(from: number, to: number) {
  return create(chat.ListMessagesResponseSchema, {
    messages: ALL.slice(from - 1, to),
    olderCursor: `before:${from}`,
    hasOlder: from > 1,
    newerCursor: `after:${to}`,
    hasNewer: to < 60,
    readCursors: [{ userId: "aoi", readSeq: 55n }],
  });
}

async function mockServer(input: string, init: { body: Uint8Array }): Promise<Response> {
  const method = new URL(input).pathname.split("/").pop();
  if (method !== "ListMessages") throw new Error(`unexpected ${method}`);
  const request = fromBinary(chat.ListMessagesRequestSchema, init.body);
  const position = request.position;
  asked.push(`${position.case}:${position.value ?? ""}`);
  const answer =
    position.case === "before"
      ? page(Number(position.value.split(":")[1]) - 20, Number(position.value.split(":")[1]) - 1)
      : page(41, 60);
  return new Response(toBinary(chat.ListMessagesResponseSchema, answer), {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array }) => mockServer(input, init),
}));

const me = { id: "me", handle: "me", displayName: "me" } as never;
const conversation = create(chat.ConversationSchema, {
  id: "c1",
  kind: chat.ConversationKind.DIRECT,
  lastSeq: 60n,
  viewer: { requestState: chat.RequestState.ACCEPTED, readSeq: 60n },
});

let queryClient: QueryClient;
// Nothing of a test (garbage-collection timers of the cache) outlives it.
afterEach(() => queryClient.clear());

function render() {
  queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return renderHook(() => useMessageWindow("c1", conversation, me), { wrapper });
}

const seqs = (rows: ReturnType<typeof useMessageWindow>["rows"]) =>
  rows.flatMap((row) => (row.type === "message" ? [Number(row.message.seq)] : []));

beforeEach(() => {
  asked.length = 0;
  useLive.getState().reset();
  useOutbox.setState({ items: {} });
});

describe("the message window", () => {
  it("opens on the newest page, and keeps the read cursors it brought", async () => {
    const { result } = await render();
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(seqs(result.current.rows)).toEqual(Array.from({ length: 20 }, (_, i) => 41 + i));
    expect(asked).toEqual(["aroundUnread:true"]);
    expect(useLive.getState().cursors.c1?.aoi).toBe(55n);
  });

  it("pages older messages in at the top, once however often scrolling asks", async () => {
    const { result } = await render();
    await waitFor(() => expect(result.current.loading).toBe(false));
    await act(async () => {
      result.current.loadOlder();
      result.current.loadOlder();
    });
    await waitFor(() => expect(seqs(result.current.rows)[0]).toBe(21));
    expect(asked).toEqual(["aroundUnread:true", "before:before:41"]);
    expect(seqs(result.current.rows)).toHaveLength(40);
    expect(result.current.hasOlder).toBe(true);

    await act(async () => result.current.loadOlder());
    await waitFor(() => expect(seqs(result.current.rows)[0]).toBe(1));
    expect(result.current.hasOlder).toBe(false);
    await act(async () => result.current.loadOlder());
    expect(asked).toHaveLength(3);
  });

  it("draws a message on its way after the newest one", async () => {
    const { result } = await render();
    await waitFor(() => expect(result.current.loading).toBe(false));
    await act(async () =>
      useOutbox.setState({
        items: {
          c1: [
            {
              clientId: "k1",
              conversationId: "c1",
              createdAt: Date.now(),
              text: "on its way",
              photos: [],
              status: "sending",
              progress: 0,
            },
          ],
        },
      }),
    );
    const last = result.current.rows.at(-1);
    expect(last?.type === "message" && last.message.text).toBe("on its way");
    expect(last?.type === "message" && last.pending?.status).toBe("sending");
  });
});
