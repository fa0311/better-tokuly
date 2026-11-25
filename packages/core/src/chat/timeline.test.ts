import { chat, create, fromDate } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  buildRows,
  leavesGap,
  mergeMessage,
  type Row,
  reactionsAfter,
  rowKey,
  type Timeline,
  timelineOf,
  withCreated,
  withNewer,
  withOlder,
  withReaction,
  withUpdated,
} from "./timeline";

const ME = "me";
const AOI = "aoi";
/** 3 November 2026, 10:00 local time: the tests place messages relative to it. */
const BASE = new Date(2026, 10, 3, 10, 0, 0).getTime();

function message(
  seq: number,
  options: {
    sender?: string;
    text?: string;
    minutes?: number;
    kind?: chat.MessageKind;
    clientId?: string;
  } = {},
): chat.Message {
  const sender = options.sender ?? AOI;
  return create(chat.MessageSchema, {
    id: `m${seq}`,
    conversationId: "c1",
    seq: BigInt(seq),
    sender:
      options.kind === chat.MessageKind.SYSTEM
        ? undefined
        : { id: sender, handle: sender, displayName: sender },
    kind: options.kind ?? chat.MessageKind.TEXT,
    clientId: options.clientId ?? "",
    text: options.text ?? `text ${seq}`,
    createdAt: fromDate(new Date(BASE + (options.minutes ?? seq) * 60_000)),
  });
}

function timeline(messages: chat.Message[], extra: Partial<Timeline> = {}): Timeline {
  return {
    messages,
    olderCursor: "older",
    hasOlder: false,
    newerCursor: "newer",
    hasNewer: false,
    ...extra,
  };
}

const seqs = (held: Timeline) => held.messages.map((item) => Number(item.seq));

describe("timeline", () => {
  it("keeps messages oldest first whatever order a page arrives in", () => {
    expect(
      seqs(
        timelineOf({
          messages: [message(3), message(1), message(2)],
          olderCursor: "",
          hasOlder: false,
          newerCursor: "",
          hasNewer: false,
        }),
      ),
    ).toEqual([1, 2, 3]);
  });

  it("joins an older page in front and takes over its cursor", () => {
    const next = withOlder(timeline([message(5), message(6)], { hasOlder: true }), {
      messages: [message(3), message(4), message(5)],
      olderCursor: "older-2",
      hasOlder: false,
      newerCursor: "x",
      hasNewer: true,
    });
    expect(seqs(next)).toEqual([3, 4, 5, 6]);
    expect(next).toMatchObject({
      olderCursor: "older-2",
      hasOlder: false,
      newerCursor: "newer",
      hasNewer: false,
    });
  });

  it("joins a newer page behind and takes over its cursor", () => {
    const next = withNewer(timeline([message(1), message(2)], { hasNewer: true }), {
      messages: [message(2), message(3)],
      olderCursor: "x",
      hasOlder: true,
      newerCursor: "newer-2",
      hasNewer: false,
    });
    expect(seqs(next)).toEqual([1, 2, 3]);
    expect(next).toMatchObject({ newerCursor: "newer-2", hasNewer: false, olderCursor: "older" });
  });

  it("appends a new message only at the live end", () => {
    expect(seqs(withCreated(timeline([message(1)]), message(2)))).toEqual([1, 2]);
    const reading = timeline([message(1)], { hasNewer: true });
    expect(withCreated(reading, message(9))).toBe(reading);
  });

  it("does not show a message twice when its event and its response both arrive", () => {
    const once = withCreated(timeline([message(1)]), message(2, { text: "hi" }));
    expect(seqs(withCreated(once, message(2, { text: "hi" })))).toEqual([1, 2]);
  });

  it("notices a hole between the newest message held and one that arrives", () => {
    expect(leavesGap(timeline([message(1), message(2)]), message(3))).toBe(false);
    expect(leavesGap(timeline([message(1), message(2)]), message(5))).toBe(true);
    expect(leavesGap(timeline([]), message(5))).toBe(false);
  });

  it("keeps what only the viewer knows when an event replaces a message", () => {
    const held = {
      ...message(1),
      readCount: 3,
      reactions: [create(chat.ReactionCountSchema, { emoji: "❤️", count: 2, reactedByMe: true })],
    };
    const fromEvent = {
      ...message(1, { text: "edited" }),
      reactions: [
        create(chat.ReactionCountSchema, { emoji: "❤️", count: 2 }),
        create(chat.ReactionCountSchema, { emoji: "😂", count: 1 }),
      ],
    };
    const merged = mergeMessage(held, fromEvent);
    expect(merged.text).toBe("edited");
    expect(merged.readCount).toBe(3);
    expect(merged.reactions.map((reaction) => [reaction.emoji, reaction.reactedByMe])).toEqual([
      ["❤️", true],
      ["😂", false],
    ]);
  });

  it("replaces an updated message and leaves unknown ones alone", () => {
    const held = timeline([message(1), message(2)]);
    expect(withUpdated(held, message(2, { text: "new" }), false).messages[1]?.text).toBe("new");
    expect(withUpdated(held, message(7), false)).toBe(held);
  });

  it("counts reactions as people add and remove them", () => {
    const added = reactionsAfter([], { emoji: "👍", added: true, count: 1, byMe: true });
    expect(added.map((reaction) => [reaction.emoji, reaction.count, reaction.reactedByMe])).toEqual(
      [["👍", 1, true]],
    );
    // Someone else joins: the count grows, "mine" stays.
    const joined = reactionsAfter(added, { emoji: "👍", added: true, count: 2, byMe: false });
    expect(joined[0]).toMatchObject({ count: 2, reactedByMe: true });
    // I take mine back; theirs remains.
    const left = reactionsAfter(joined, { emoji: "👍", added: false, count: 1, byMe: true });
    expect(left[0]).toMatchObject({ count: 1, reactedByMe: false });
    expect(reactionsAfter(left, { emoji: "👍", added: false, count: 0, byMe: false })).toEqual([]);
  });

  it("applies a reaction change to its message only", () => {
    const next = withReaction(timeline([message(1), message(2)]), "m2", {
      emoji: "🔥",
      added: true,
      count: 1,
      byMe: false,
    });
    expect(next.messages.map((item) => item.reactions.length)).toEqual([0, 1]);
  });
});

describe("rows", () => {
  const types = (rows: Row[]) =>
    rows.map((row) => (row.type === "message" ? `m${row.message.seq}` : row.type));

  it("starts each day with a separator", () => {
    const rows = buildRows(
      [
        message(1, { minutes: 0 }),
        message(2, { minutes: 24 * 60 }),
        message(3, { minutes: 24 * 60 + 1 }),
      ],
      { myId: ME },
    );
    expect(types(rows)).toEqual(["day", "m1", "day", "m2", "m3"]);
  });

  it("draws messages by one person close together as a run", () => {
    const rows = buildRows(
      [
        message(1, { minutes: 0 }),
        message(2, { minutes: 1 }),
        message(3, { minutes: 2, sender: ME }),
        message(4, { minutes: 30, sender: ME }),
      ],
      { myId: ME },
    );
    const flags = rows.flatMap((row) =>
      row.type === "message" ? [[Number(row.message.seq), row.first, row.last]] : [],
    );
    expect(flags).toEqual([
      [1, true, false],
      [2, false, true],
      [3, true, true],
      // Half an hour later: a run of its own.
      [4, true, true],
    ]);
  });

  it("never joins notices into a run", () => {
    const rows = buildRows(
      [
        message(1, { minutes: 0 }),
        message(2, { minutes: 1, kind: chat.MessageKind.SYSTEM }),
        message(3, { minutes: 2 }),
      ],
      { myId: ME },
    );
    expect(rows.flatMap((row) => (row.type === "message" ? [row.first && row.last] : []))).toEqual([
      true,
      true,
      true,
    ]);
  });

  it("marks where the unread messages begin: the first one after the read position that someone else sent", () => {
    const rows = buildRows(
      [
        message(1, { minutes: 0 }),
        message(2, { minutes: 1, sender: ME }),
        message(3, { minutes: 2, sender: ME }),
        message(4, { minutes: 3 }),
        message(5, { minutes: 4 }),
      ],
      { myId: ME, unreadAfterSeq: 2n },
    );
    expect(types(rows)).toEqual(["day", "m1", "m2", "m3", "unread", "m4", "m5"]);
    // The message after the divider starts a new run.
    expect(rows.find((row) => row.type === "message" && row.message.seq === 4n)).toMatchObject({
      first: true,
    });
  });

  it("leaves the divider out when nothing was unread", () => {
    expect(types(buildRows([message(1, { minutes: 0 })], { myId: ME }))).toEqual(["day", "m1"]);
  });

  it("puts messages still being sent at the end, under the identity their stored copy will have", () => {
    const sending = {
      ...message(99, { sender: ME, clientId: "abc", minutes: 5 }),
      id: "pending:abc",
    };
    const rows = buildRows([message(1, { minutes: 0 })], {
      myId: ME,
      pending: [{ message: sending, state: { status: "sending" } }],
    });
    const last = rows.at(-1);
    expect(last).toMatchObject({ type: "message", pending: { status: "sending" } });
    expect(last?.key).toBe(rowKey(message(7, { sender: ME, clientId: "abc" })));
  });
});
