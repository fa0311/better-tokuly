import { chat, create, toMillis } from "@tokuly/api";

/**
 * The messages of one conversation the client holds: one unbroken stretch,
 * oldest first, with the cursors to continue in both directions. It is at the
 * "live end" while `hasNewer` is false: new messages are then appended as
 * they arrive.
 */
export interface Timeline {
  messages: chat.Message[];
  olderCursor: string;
  hasOlder: boolean;
  newerCursor: string;
  hasNewer: boolean;
}

/** One page of `ListMessages`, reduced to what a timeline keeps. */
export type MessagePage = Pick<
  chat.ListMessagesResponse,
  "messages" | "olderCursor" | "hasOlder" | "newerCursor" | "hasNewer"
>;

export function timelineOf(page: MessagePage): Timeline {
  return {
    messages: sorted(page.messages),
    olderCursor: page.olderCursor,
    hasOlder: page.hasOlder,
    newerCursor: page.newerCursor,
    hasNewer: page.hasNewer,
  };
}

function sorted(messages: chat.Message[]): chat.Message[] {
  return [...messages].sort((a, b) => (a.seq < b.seq ? -1 : a.seq > b.seq ? 1 : 0));
}

/** Joins two stretches, the copy in `fresh` winning for messages in both. */
function union(held: chat.Message[], fresh: chat.Message[]): chat.Message[] {
  const byId = new Map(held.map((message) => [message.id, message]));
  for (const message of fresh) byId.set(message.id, message);
  return sorted([...byId.values()]);
}

/** Adds a page fetched with `before: olderCursor`. */
export function withOlder(timeline: Timeline, page: MessagePage): Timeline {
  return {
    ...timeline,
    messages: union(timeline.messages, page.messages),
    olderCursor: page.olderCursor,
    hasOlder: page.hasOlder,
  };
}

/** Adds a page fetched with `after: newerCursor`. */
export function withNewer(timeline: Timeline, page: MessagePage): Timeline {
  return {
    ...timeline,
    messages: union(timeline.messages, page.messages),
    newerCursor: page.newerCursor,
    hasNewer: page.hasNewer,
  };
}

/** Whether a message that just arrived would leave a hole behind the newest one held. */
export function leavesGap(timeline: Timeline, message: chat.Message): boolean {
  const newest = timeline.messages.at(-1);
  return Boolean(newest) && message.seq > newest!.seq + 1n;
}

/**
 * Adds a message that was just created. Nothing changes while the timeline
 * is not at the live end (the reader is somewhere in the history; the message
 * comes with the pages on the way back).
 */
export function withCreated(timeline: Timeline, message: chat.Message): Timeline {
  if (timeline.hasNewer) return timeline;
  const held = timeline.messages.find((candidate) => candidate.id === message.id);
  if (held)
    return {
      ...timeline,
      messages: timeline.messages.map((candidate) =>
        candidate === held ? mergeMessage(held, message) : candidate,
      ),
    };
  return { ...timeline, messages: sorted([...timeline.messages, message]) };
}

/**
 * The stored copy of a message after an event about it. Events are built for
 * everyone, so what only this viewer knows (how many read it, which reactions
 * are theirs) is carried over from the copy already held.
 */
export function mergeMessage(held: chat.Message, incoming: chat.Message): chat.Message {
  const mine = new Set(
    held.reactions.filter((reaction) => reaction.reactedByMe).map((reaction) => reaction.emoji),
  );
  return {
    ...incoming,
    readCount: Math.max(held.readCount, incoming.readCount),
    reactions: incoming.reactions.map((reaction) =>
      mine.has(reaction.emoji) && !reaction.reactedByMe
        ? { ...reaction, reactedByMe: true }
        : reaction,
    ),
  };
}

/** Replaces a message with its changed version; unknown messages are ignored. */
export function withUpdated(
  timeline: Timeline,
  message: chat.Message,
  fromEvent: boolean,
): Timeline {
  if (!timeline.messages.some((held) => held.id === message.id)) return timeline;
  return {
    ...timeline,
    messages: timeline.messages.map((held) =>
      held.id === message.id ? (fromEvent ? mergeMessage(held, message) : message) : held,
    ),
  };
}

/** The reactions of a message after one person added or removed an emoji. */
export function reactionsAfter(
  reactions: chat.ReactionCount[],
  change: { emoji: string; added: boolean; count: number; byMe: boolean },
): chat.ReactionCount[] {
  const held = reactions.find((reaction) => reaction.emoji === change.emoji);
  const reactedByMe = change.byMe ? change.added : (held?.reactedByMe ?? false);
  if (change.count <= 0) return reactions.filter((reaction) => reaction.emoji !== change.emoji);
  if (!held)
    return [
      ...reactions,
      create(chat.ReactionCountSchema, { emoji: change.emoji, count: change.count, reactedByMe }),
    ];
  return reactions.map((reaction) =>
    reaction === held ? { ...reaction, count: change.count, reactedByMe } : reaction,
  );
}

/** Applies a reaction change to the message it belongs to. */
export function withReaction(
  timeline: Timeline,
  messageId: string,
  change: { emoji: string; added: boolean; count: number; byMe: boolean },
): Timeline {
  if (!timeline.messages.some((held) => held.id === messageId)) return timeline;
  return {
    ...timeline,
    messages: timeline.messages.map((held) =>
      held.id === messageId ? { ...held, reactions: reactionsAfter(held.reactions, change) } : held,
    ),
  };
}

// ───────────────────────────── Rows ─────────────────────────────

/** Local sending state of a message that is not on the server yet. */
export interface PendingState {
  status: "sending" | "failed";
  /** 0..1 while attachments upload. */
  progress?: number;
  /** Why it failed, ready to show. */
  error?: string;
}

/** One line of the conversation as drawn. */
export type Row =
  | { type: "day"; key: string; at: number }
  | { type: "unread"; key: string }
  | {
      type: "message";
      key: string;
      message: chat.Message;
      /** First of a run by the same sender: gets the name and avatar. */
      first: boolean;
      /** Last of a run: gets the time and the bubble's tail. */
      last: boolean;
      pending?: PendingState;
    };

/**
 * A row's identity. A message is known by its sender's idempotency key where
 * it has one, so the bubble shown while sending is the same row as the stored
 * message that replaces it.
 */
export function rowKey(message: chat.Message): string {
  return message.clientId && message.sender
    ? `c:${message.sender.id}:${message.clientId}`
    : message.id;
}

/** Messages by the same person less than this apart are drawn as one run. */
const RUN_GAP_MS = 3 * 60_000;

function dayOf(ms: number): string {
  const date = new Date(ms);
  return `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`;
}

/** Whether a message is drawn as a bubble of its sender (and not as a centred notice). */
export function isBubble(message: chat.Message): boolean {
  return (
    message.kind !== chat.MessageKind.SYSTEM &&
    message.kind !== chat.MessageKind.CALL &&
    message.kind !== chat.MessageKind.POV_REQUEST
  );
}

function sameRun(a: chat.Message, b: chat.Message): boolean {
  return (
    isBubble(a) &&
    isBubble(b) &&
    a.sender?.id === b.sender?.id &&
    Math.abs(toMillis(b.createdAt) - toMillis(a.createdAt)) < RUN_GAP_MS &&
    dayOf(toMillis(a.createdAt)) === dayOf(toMillis(b.createdAt))
  );
}

/**
 * Lays the messages out as rows: a day separator where the date changes, the
 * "unread from here" divider before the first message after `unreadAfterSeq`
 * that someone else sent, and runs of bubbles by the same sender.
 */
export function buildRows(
  messages: chat.Message[],
  options: {
    pending?: { message: chat.Message; state: PendingState }[];
    unreadAfterSeq?: bigint;
    myId: string;
  },
): Row[] {
  const rows: Row[] = [];
  const all: { message: chat.Message; pending?: PendingState }[] = [
    ...messages.map((message) => ({ message })),
    ...(options.pending ?? []).map((entry) => ({ message: entry.message, pending: entry.state })),
  ];
  let day = "";
  let dividerPlaced = options.unreadAfterSeq === undefined;

  all.forEach((entry, index) => {
    const { message, pending } = entry;
    const at = toMillis(message.createdAt);
    const messageDay = dayOf(at);
    if (messageDay !== day) {
      day = messageDay;
      rows.push({ type: "day", key: `day:${messageDay}`, at });
    }
    if (
      !dividerPlaced &&
      !pending &&
      message.seq > options.unreadAfterSeq! &&
      message.sender?.id !== options.myId &&
      message.kind !== chat.MessageKind.SYSTEM
    ) {
      dividerPlaced = true;
      rows.push({ type: "unread", key: "unread" });
    }
    const previous = all[index - 1]?.message;
    const next = all[index + 1]?.message;
    const afterDivider = rows.at(-1)?.type !== "message";
    rows.push({
      type: "message",
      key: rowKey(message),
      message,
      first: afterDivider || !previous || !sameRun(previous, message),
      last: !next || !sameRun(message, next),
      pending,
    });
  });
  return rows;
}
