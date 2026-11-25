import { chat, create, fromDate, type MessageInitShape } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { make } from "../testing/make";
import {
  abilitiesOf,
  compareConversations,
  isMuted,
  listOf,
  mentionsUser,
  peerOf,
  ringsFor,
} from "./model";

const user = (id: string) => ({ id, handle: id, displayName: id });

function conversation(
  id: string,
  viewer: MessageInitShape<typeof chat.ViewerStateSchema> = {},
  extra: MessageInitShape<typeof chat.ConversationSchema> = {},
): chat.Conversation {
  return make(
    chat.ConversationSchema,
    {
      id,
      kind: chat.ConversationKind.DIRECT,
      members: [user("me"), user("aoi")],
      lastSeq: 3n,
      lastActivityAt: fromDate(new Date(2026, 10, 3, 10)),
      viewer: make(
        chat.ViewerStateSchema,
        { role: chat.MemberRole.MEMBER, requestState: chat.RequestState.ACCEPTED },
        viewer,
      ),
    },
    extra,
  );
}

describe("conversations", () => {
  it("finds the other person of a direct conversation", () => {
    expect(peerOf(conversation("c"), "me")?.id).toBe("aoi");
    expect(
      peerOf(conversation("c", {}, { kind: chat.ConversationKind.GROUP }), "me"),
    ).toBeUndefined();
  });

  it("sorts each conversation into the list the server shows it in", () => {
    expect(listOf(conversation("c"))).toBe("inbox");
    expect(listOf(conversation("c", { archived: true }))).toBe("archived");
    expect(listOf(conversation("c", { requestState: chat.RequestState.PENDING }))).toBe("requests");
    expect(listOf(conversation("c", { requestState: chat.RequestState.DECLINED }))).toBeNull();
    expect(listOf(conversation("c", { hidden: true }))).toBeNull();
    // A direct conversation nobody wrote in is not listed; an empty group is.
    expect(listOf(conversation("c", {}, { lastSeq: 0n }))).toBeNull();
    expect(listOf(conversation("c", {}, { lastSeq: 0n, kind: chat.ConversationKind.GROUP }))).toBe(
      "inbox",
    );
  });

  it("orders pinned conversations first, then the latest activity", () => {
    const old = conversation("old", {}, { lastActivityAt: fromDate(new Date(2026, 10, 1)) });
    const fresh = conversation("fresh", {}, { lastActivityAt: fromDate(new Date(2026, 10, 3)) });
    const pinned = conversation(
      "pinned",
      { pinned: true },
      { lastActivityAt: fromDate(new Date(2026, 9, 1)) },
    );
    expect([old, fresh, pinned].sort(compareConversations).map((item) => item.id)).toEqual([
      "pinned",
      "fresh",
      "old",
    ]);
  });

  it("lets a timed mute run out by itself", () => {
    const now = Date.now();
    expect(isMuted(create(chat.ViewerStateSchema, { muted: true }), now)).toBe(true);
    expect(
      isMuted(
        create(chat.ViewerStateSchema, {
          muted: true,
          mutedUntil: fromDate(new Date(now + 60_000)),
        }),
        now,
      ),
    ).toBe(true);
    expect(
      isMuted(
        create(chat.ViewerStateSchema, {
          muted: true,
          mutedUntil: fromDate(new Date(now - 60_000)),
        }),
        now,
      ),
    ).toBe(false);
    expect(isMuted(undefined, now)).toBe(false);
  });
});

describe("messages", () => {
  const now = new Date(2026, 10, 3, 10, 0, 0).getTime();
  const text = (sender: string, extra: MessageInitShape<typeof chat.MessageSchema> = {}) =>
    make(
      chat.MessageSchema,
      {
        id: "m",
        sender: user(sender),
        kind: chat.MessageKind.TEXT,
        text: "こんにちは",
        createdAt: fromDate(new Date(now - 60_000)),
      },
      extra,
    );
  const context = {
    myId: "me",
    role: chat.MemberRole.MEMBER,
    writable: true,
    accepted: true,
    editWindowSeconds: 900,
    nowMs: now,
  };

  it("lets people edit and unsend their own messages, within the edit window", () => {
    expect(abilitiesOf(text("me"), context)).toMatchObject({
      edit: true,
      delete: true,
      reply: true,
      react: true,
      copy: true,
      forward: true,
      pin: true,
    });
    expect(
      abilitiesOf(text("me", { createdAt: fromDate(new Date(now - 16 * 60_000)) }), context),
    ).toMatchObject({ edit: false, delete: true });
    expect(abilitiesOf(text("me", { forwarded: true }), context).edit).toBe(false);
  });

  it("keeps other people's messages theirs, except for a group's admins", () => {
    expect(abilitiesOf(text("aoi"), context)).toMatchObject({
      edit: false,
      delete: false,
      reply: true,
    });
    expect(abilitiesOf(text("aoi"), { ...context, role: chat.MemberRole.ADMIN })).toMatchObject({
      edit: false,
      delete: true,
    });
  });

  it("offers nothing on tombstones, notices and in conversations that cannot be written in", () => {
    const none = {
      reply: false,
      react: false,
      copy: false,
      edit: false,
      forward: false,
      pin: false,
      delete: false,
      translate: false,
    };
    expect(abilitiesOf(text("me", { deleted: true }), context)).toEqual(none);
    expect(abilitiesOf(text("me", { kind: chat.MessageKind.SYSTEM }), context)).toEqual(none);
    expect(abilitiesOf(text("aoi"), { ...context, writable: false })).toMatchObject({
      reply: false,
      react: false,
      pin: false,
      copy: true,
    });
    // A message request that was not accepted yet: no reactions or pins.
    expect(abilitiesOf(text("aoi"), { ...context, accepted: false })).toMatchObject({
      react: false,
      pin: false,
      reply: true,
    });
  });

  it("does not offer to forward a wave or the answer to a POV request", () => {
    expect(
      abilitiesOf(text("me", { kind: chat.MessageKind.HELLO, text: "" }), context).forward,
    ).toBe(false);
    expect(
      abilitiesOf(
        text("me", { kind: chat.MessageKind.MEDIA, povReply: { requestId: "p" } }),
        context,
      ),
    ).toMatchObject({ forward: false, edit: false });
  });

  it("knows who a message mentions", () => {
    expect(
      mentionsUser(text("aoi", { mentions: [{ handle: "me", user: user("me") }] }), "me"),
    ).toBe(true);
    expect(mentionsUser(text("aoi", { mentionsEveryone: true }), "me")).toBe(true);
    expect(mentionsUser(text("aoi"), "me")).toBe(false);
  });
});

describe("calls", () => {
  const call = (state: chat.CallState, mine: chat.CallParticipantState, startedBy = "aoi") =>
    create(chat.CallSchema, {
      id: "call",
      state,
      startedBy: user(startedBy),
      participants: [
        { user: user("aoi"), state: chat.CallParticipantState.JOINED },
        { user: user("me"), state: mine },
      ],
    });

  it("rings only while the call rings for this person", () => {
    expect(ringsFor(call(chat.CallState.RINGING, chat.CallParticipantState.RINGING), "me")).toBe(
      true,
    );
    // Answered or declined on another device, or the call moved on.
    expect(ringsFor(call(chat.CallState.RINGING, chat.CallParticipantState.DECLINED), "me")).toBe(
      false,
    );
    expect(ringsFor(call(chat.CallState.ACTIVE, chat.CallParticipantState.JOINED), "me")).toBe(
      false,
    );
    expect(ringsFor(call(chat.CallState.ENDED, chat.CallParticipantState.MISSED), "me")).toBe(
      false,
    );
    // Never for the person who started it.
    expect(
      ringsFor(call(chat.CallState.RINGING, chat.CallParticipantState.RINGING, "me"), "me"),
    ).toBe(false);
  });
});
