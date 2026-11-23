import { create } from "@bufbuild/protobuf";
import { common, fromDate, spaces } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  mergeHangout,
  mergeSpace,
  minutesOf,
  parseTags,
  withoutParticipant,
  withParticipant,
  withPresence,
} from "./model";
import { clockOf, minuteOf, suggestedStart } from "./when";

const Role = spaces.SpaceRole;
const user = (id: string) => create(common.UserSummarySchema, { id, handle: id, displayName: id });
const participant = (id: string, role: spaces.SpaceRole, joined = 0) =>
  create(spaces.SpaceParticipantSchema, {
    user: user(id),
    role,
    joinedAt: fromDate(new Date(joined)),
  });

function room(fields: Omit<Partial<spaces.Space>, "$typeName"> = {}): spaces.Space {
  return create(spaces.SpaceSchema, {
    id: "s1",
    state: spaces.SpaceState.LIVE,
    host: user("host"),
    stage: [participant("host", Role.HOST)],
    viewer: create(spaces.SpaceViewerSchema, { inside: true, role: Role.LISTENER }),
    ...fields,
  });
}

describe("tags", () => {
  it("takes what people type: hashes, commas, Japanese separators, repeats", () => {
    expect(parseTags("#部活, ゲーム  雑談、＃部活 ")).toEqual(["部活", "ゲーム", "雑談"]);
    expect(parseTags("   ")).toEqual([]);
  });
});

describe("the stage", () => {
  it("a listener who comes up joins the stage after the host; the viewer's own role follows", () => {
    const next = withParticipant(room(), participant("me", Role.SPEAKER, 5), "me");
    expect(next.stage.map((entry) => entry.user?.id)).toEqual(["host", "me"]);
    expect(next.viewer?.role).toBe(Role.SPEAKER);
    expect(next.viewer?.stageInvited).toBe(false);
  });

  it("co-hosts sit before speakers, and stepping down leaves the stage", () => {
    let next = withParticipant(room(), participant("a", Role.SPEAKER, 1), "me");
    next = withParticipant(next, participant("b", Role.COHOST, 2), "me");
    expect(next.stage.map((entry) => entry.user?.id)).toEqual(["host", "b", "a"]);
    next = withParticipant(next, participant("a", Role.LISTENER, 1), "me");
    expect(next.stage.map((entry) => entry.user?.id)).toEqual(["host", "b"]);
  });

  it("leaving takes the person off the stage, and the viewer outside", () => {
    const staged = withParticipant(room(), participant("me", Role.SPEAKER), "me");
    const next = withoutParticipant(staged, "me", "me");
    expect(next.stage).toHaveLength(1);
    expect(next.viewer?.inside).toBe(false);
  });

  it("a pushed room keeps the viewer's relation, demoting a speaker the stage no longer lists", () => {
    const loaded = withParticipant(room(), participant("me", Role.SPEAKER), "me");
    const merged = mergeSpace(loaded, room({ viewer: undefined }), "me");
    expect(merged.viewer?.role).toBe(Role.LISTENER);
    expect(merged.viewer?.inside).toBe(true);
    const ended = mergeSpace(
      loaded,
      room({ state: spaces.SpaceState.ENDED, viewer: undefined }),
      "me",
    );
    expect(ended.viewer?.inside).toBe(false);
  });

  it("counts whole minutes, at least one", () => {
    const started = fromDate(new Date(1_000_000));
    expect(minutesOf(room({ startedAt: started, endedAt: fromDate(new Date(1_010_000)) }))).toBe(1);
    expect(
      minutesOf(
        room({ startedAt: started, endedAt: fromDate(new Date(1_000_000 + 150 * 60_000)) }),
      ),
    ).toBe(150);
  });
});

describe("hangouts", () => {
  const member = (id: string, inside = false) =>
    create(spaces.HangoutMemberSchema, {
      user: user(id),
      state: spaces.HangoutMemberState.MEMBER,
      inside,
    });
  const hangout = create(spaces.HangoutSchema, {
    id: "h1",
    owner: user("owner"),
    members: [member("owner"), member("me")],
    viewer: create(spaces.HangoutViewerSchema, {
      state: spaces.HangoutMemberState.MEMBER,
      inside: false,
    }),
  });

  it("someone entering leads the list, and the count follows", () => {
    const presence = create(spaces.HangoutPresenceSchema, {
      hangoutId: "h1",
      user: user("me"),
      inside: true,
      insideCount: 1,
    });
    const next = withPresence(hangout, presence, "me", undefined);
    expect(next.members[0]?.user?.id).toBe("me");
    expect(next.insideCount).toBe(1);
    expect(next.viewer?.inside).toBe(true);
  });

  it("a push without a viewer takes the viewer's state from the member list", () => {
    const pushed = create(spaces.HangoutSchema, {
      ...hangout,
      viewer: undefined,
      membersCanInvite: true,
      members: [member("owner"), member("me", true)],
    });
    const merged = mergeHangout(hangout, pushed, "me");
    expect(merged.viewer?.inside).toBe(true);
    expect(merged.viewer?.canInvite).toBe(true);
    expect(merged.viewer?.owner).toBe(false);
  });
});

describe("schedule times", () => {
  it("reads and writes the time input", () => {
    expect(clockOf(18 * 60 + 5)).toBe("18:05");
    expect(minuteOf("09:30")).toBe(570);
    expect(minuteOf("24:00")).toBeNull();
    expect(minuteOf("nine")).toBeNull();
  });

  it("proposes the next full hour at least half an hour away, on the account's clock", () => {
    // 2026-11-23 10:40 in Tokyo is 01:40 UTC.
    const now = Date.UTC(2026, 10, 23, 1, 40);
    expect(suggestedStart(now, "Asia/Tokyo")).toEqual({ date: "2026-11-23", clock: "12:00" });
    // The same moment is the day before in Pago Pago.
    expect(suggestedStart(now, "Pacific/Pago_Pago")).toEqual({
      date: "2026-11-22",
      clock: "16:00",
    });
  });
});
