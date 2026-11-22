import { chat, create, fromDate, type MessageInitShape } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { listAfter } from "./conversations";
import { make } from "./test-support";

function conversation(
  id: string,
  day: number,
  viewer: MessageInitShape<typeof chat.ViewerStateSchema> = {},
): chat.Conversation {
  return create(chat.ConversationSchema, {
    id,
    kind: chat.ConversationKind.GROUP,
    name: id,
    lastSeq: 1n,
    lastActivityAt: fromDate(new Date(2026, 10, day)),
    viewer: make(chat.ViewerStateSchema, { requestState: chat.RequestState.ACCEPTED }, viewer),
  });
}

const list = (conversations: chat.Conversation[], nextCursor = "") => ({
  pages: [{ conversations, nextCursor }],
  pageParams: [""],
});
const ids = (data: ReturnType<typeof list>) =>
  data.pages.flatMap((page) => page.conversations.map((item) => item.id));

describe("conversation lists", () => {
  it("moves a conversation with a new message to the top", () => {
    const data = list([conversation("a", 3), conversation("b", 2), conversation("c", 1)]);
    expect(ids(listAfter(data, "inbox", conversation("c", 4)))).toEqual(["c", "a", "b"]);
  });

  it("keeps pinned conversations above newer ones", () => {
    const data = list([conversation("pinned", 1, { pinned: true }), conversation("b", 2)]);
    expect(ids(listAfter(data, "inbox", conversation("new", 5)))).toEqual(["pinned", "new", "b"]);
  });

  it("takes a conversation out of the list it no longer belongs in", () => {
    const data = list([conversation("a", 3), conversation("b", 2)]);
    expect(ids(listAfter(data, "inbox", conversation("b", 2, { archived: true })))).toEqual(["a"]);
    expect(ids(listAfter(list([]), "archived", conversation("b", 2, { archived: true })))).toEqual([
      "b",
    ]);
  });

  it("leaves a conversation older than everything loaded to the pages still to come", () => {
    const data = list([conversation("a", 3), conversation("b", 2)], "more");
    expect(ids(listAfter(data, "inbox", conversation("ancient", 1)))).toEqual(["a", "b"]);
    // With nothing more to load, it belongs at the end.
    expect(
      ids(listAfter(list([conversation("a", 3)]), "inbox", conversation("ancient", 1))),
    ).toEqual(["a", "ancient"]);
  });

  it("changes nothing when nothing changed", () => {
    const a = conversation("a", 3);
    const data = list([a, conversation("b", 2)]);
    expect(listAfter(data, "inbox", a)).toBe(data);
  });
});
