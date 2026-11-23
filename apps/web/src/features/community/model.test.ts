import { community } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  appPathOf,
  assignableRoles,
  canRead,
  checkInOpen,
  eventPhase,
  inviteCodeOf,
  parseTags,
  placesLeft,
  safeColor,
  splitMentions,
  uniqueById,
} from "./model";

const R = community.MemberRole;
const at = (ms: number) => ({ seconds: BigInt(ms / 1000), nanos: 0 }) as never;

describe("appPathOf", () => {
  it("maps the domain's paths to this app's community pages", () => {
    expect(appPathOf("/c/neko")).toBe("/g/neko");
    expect(appPathOf("/c/neko/t/bXou6FpLkfK")).toBe("/g/neko/t/bXou6FpLkfK");
    expect(appPathOf("/c/neko/events/0199-ab")).toBe("/g/neko/events/0199-ab");
    expect(appPathOf("https://tokuly.example/c/join/UwVvf0F6yd")).toBe("/join/UwVvf0F6yd");
  });
  it("keeps what follows the path, and finds the staff pages notifications link to", () => {
    expect(appPathOf("/c/neko/t/bXou6FpLkfK?reply=0199-cd")).toBe(
      "/g/neko/t/bXou6FpLkfK?reply=0199-cd",
    );
    expect(appPathOf("/c/neko/requests")).toBe("/g/neko/manage/requests");
    expect(appPathOf("/c/neko/review")).toBe("/g/neko/manage/queue");
    expect(appPathOf("/c/neko/log")).toBe("/g/neko/manage/log");
  });
  it("leaves everything else alone", () => {
    expect(appPathOf("/notes/1")).toBeUndefined();
    expect(appPathOf("/c/neko/videos/extra/deep")).toBeUndefined();
    expect(appPathOf("//evil.example/c/neko")).toBeUndefined();
  });
});

describe("inviteCodeOf", () => {
  it("takes a bare code or a link", () => {
    expect(inviteCodeOf(" UwVvf0F6yd ")).toBe("UwVvf0F6yd");
    expect(inviteCodeOf("https://tokuly.example/join/UwVvf0F6yd")).toBe("UwVvf0F6yd");
    expect(inviteCodeOf("https://tokuly.example/c/join/UwVvf0F6yd?x=1")).toBe("UwVvf0F6yd");
  });
});

describe("assignableRoles", () => {
  it("lets the owner appoint admins and moderators", () => {
    expect(assignableRoles(R.OWNER, R.MEMBER)).toEqual([R.MODERATOR, R.ADMIN]);
    expect(assignableRoles(R.OWNER, R.ADMIN)).toEqual([R.MEMBER, R.MODERATOR]);
  });
  it("lets admins appoint and dismiss moderators only", () => {
    expect(assignableRoles(R.ADMIN, R.MEMBER)).toEqual([R.MODERATOR]);
    expect(assignableRoles(R.ADMIN, R.MODERATOR)).toEqual([R.MEMBER]);
    expect(assignableRoles(R.ADMIN, R.ADMIN)).toEqual([]);
    expect(assignableRoles(R.MODERATOR, R.MEMBER)).toEqual([]);
  });
});

describe("posts", () => {
  it("normalizes tags", () => {
    expect(parseTags("#Neko, 犬　＃neko  bird", 5)).toEqual(["neko", "犬", "bird"]);
    expect(parseTags("a b c", 2)).toEqual(["a", "b"]);
  });
  it("links only the mentions the server recognised", () => {
    expect(splitMentions("hi @aoi and @nobody!", [{ handle: "aoi" }])).toEqual([
      { text: "hi " },
      { text: "@aoi", handle: "aoi" },
      { text: " and @nobody!" },
    ]);
    expect(splitMentions("plain", [])).toEqual([{ text: "plain" }]);
  });
  it("drops threads a moving hot list repeats", () => {
    expect(uniqueById([{ id: "a" }, { id: "b" }, { id: "a" }])).toEqual([{ id: "a" }, { id: "b" }]);
  });
  it("only lets colours into styles", () => {
    expect(safeColor("#12abEF")).toBe("#12abEF");
    expect(safeColor("red; background:url(x)")).toBeUndefined();
  });
});

describe("events", () => {
  const event = { startsAt: at(10 * 3_600_000), endsAt: at(12 * 3_600_000), cancelled: false };
  it("knows the phase", () => {
    expect(eventPhase(event, 9 * 3_600_000)).toBe("upcoming");
    expect(eventPhase(event, 11 * 3_600_000)).toBe("running");
    expect(eventPhase(event, 13 * 3_600_000)).toBe("over");
  });
  it("opens check-in an hour before and closes it six hours after", () => {
    expect(checkInOpen(event, 8.5 * 3_600_000)).toBe(false);
    expect(checkInOpen(event, 9.5 * 3_600_000)).toBe(true);
    expect(checkInOpen(event, 17.5 * 3_600_000)).toBe(true);
    expect(checkInOpen(event, 18.5 * 3_600_000)).toBe(false);
    expect(checkInOpen({ ...event, cancelled: true }, 11 * 3_600_000)).toBe(false);
  });
  it("counts the places left", () => {
    expect(placesLeft({ capacity: 0, goingCount: 9 })).toBeUndefined();
    expect(placesLeft({ capacity: 3, goingCount: 1 })).toBe(2);
    expect(placesLeft({ capacity: 3, goingCount: 5 })).toBe(0);
  });
});

describe("canRead", () => {
  const V = community.CommunityVisibility;
  const S = community.MembershipStatus;
  const of = (visibility: community.CommunityVisibility, status = S.NONE) =>
    ({ visibility, viewer: { status } }) as unknown as community.Community;
  it("opens public communities to everyone and closed ones to members", () => {
    expect(canRead(of(V.PUBLIC))).toBe(true);
    expect(canRead(of(V.RESTRICTED))).toBe(false);
    expect(canRead(of(V.HIDDEN, S.MEMBER))).toBe(true);
    expect(canRead(of(V.RESTRICTED, S.REQUESTED), ["staff"])).toBe(false);
  });
  it("lets site moderators and admins read every community", () => {
    expect(canRead(of(V.RESTRICTED), ["moderator"])).toBe(true);
    expect(canRead(of(V.HIDDEN), ["admin"])).toBe(true);
    expect(canRead(undefined, ["admin"])).toBe(false);
  });
});
