import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, geo } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  askedForSnapshot,
  cleared,
  NO_LOCATIONS as EMPTY,
  type LiveLocations,
  withLocation,
  withoutFriend,
  withSnapshot,
} from "./live";

const location = (
  userId: string,
  online: boolean,
  { minute = 0, lat = 35.68, precision = geo.GhostMode.PRECISE } = {},
) =>
  create(geo.FriendLocationSchema, {
    userId,
    online,
    precision,
    position: { lat, lng: 139.76 },
    recordedAt: timestampFromMs(Date.UTC(2026, 10, 28, 12, minute)),
  });

/** A snapshot read when the viewer had been sent `removalSequence` numbered changes. */
const snapshot = (locations: geo.FriendLocation[], removalSequence = 0n) => ({
  locations,
  removalSequence,
});

/** A snapshot asked for now and answered at once. */
const answer = (live: LiveLocations, read: ReturnType<typeof snapshot>) => {
  const asking = askedForSnapshot(live);
  return withSnapshot(asking, read, asking.asked);
};

describe("live locations", () => {
  it("indexes a snapshot by friend and keeps only who is online", () => {
    const live = answer(EMPTY, snapshot([location("aoi", true), location("ren", false)]));
    expect(Object.keys(live.friends)).toEqual(["aoi", "ren"]);
    expect(live.online).toEqual({ aoi: true });
  });

  it("takes a friend's new location and drops one who left", () => {
    const live = withLocation(
      answer(EMPTY, snapshot([location("aoi", true)])),
      location("aoi", false),
    );
    expect(live.online.aoi).toBe(false);
    expect(withoutFriend(live, "aoi").friends).toEqual({});
  });

  it("keeps a pushed pin newer than a snapshot that arrives after it", () => {
    const pushed = withLocation(EMPTY, location("aoi", true, { minute: 5, lat: 35.7 }));
    const live = answer(
      pushed,
      snapshot([location("aoi", false, { minute: 4 }), location("ren", true, { minute: 4 })]),
    );
    expect(live.friends.aoi?.position?.lat).toBe(35.7);
    expect(live.online).toEqual({ aoi: true, ren: true });
  });

  it("takes the snapshot's pin when it is the newer one, and drops who is not in it", () => {
    const held = withLocation(EMPTY, location("aoi", true, { minute: 4 }));
    const live = answer(
      withLocation(held, location("ren", true)),
      snapshot([location("aoi", false, { minute: 5, lat: 35.7 })]),
    );
    expect(live.friends.aoi?.position?.lat).toBe(35.7);
    expect(live.online).toEqual({});
    expect(Object.keys(live.friends)).toEqual(["aoi"]);
  });

  it("keeps a blurred pin against a precise one, whose time is not rounded", () => {
    const blurred = location("aoi", true, { minute: 0, precision: geo.GhostMode.BLURRED });
    const live = answer(
      withLocation(EMPTY, blurred),
      snapshot([location("aoi", true, { minute: 3 })]),
    );
    expect(live.friends.aoi?.precision).toBe(geo.GhostMode.BLURRED);
  });

  it("keeps a friend removed after the snapshot was read off the map", () => {
    const held = answer(EMPTY, snapshot([location("aoi", true), location("ren", true)], 2n));
    const asking = askedForSnapshot(held);
    const removed = withoutFriend(asking, "aoi", 3n);
    const late = withSnapshot(
      removed,
      snapshot([location("aoi", true), location("ren", true)], 2n),
      asking.asked,
    );
    expect(Object.keys(late.friends)).toEqual(["ren"]);
    expect(late.online).toEqual({ ren: true });
    expect(Object.keys(late.removed)).toEqual(["aoi"]);
  });

  it("forgets a removal once a snapshot asked for after it is applied", () => {
    const removed = withoutFriend(withLocation(EMPTY, location("aoi", true)), "aoi", 3n);
    const read = answer(removed, snapshot([], 3n));
    expect(read.removed).toEqual({});
  });

  it("keeps a removal against a snapshot asked for before it, answering late", () => {
    const first = askedForSnapshot(withLocation(EMPTY, location("aoi", true)));
    const removed = withoutFriend(first, "aoi", 3n);
    const second = askedForSnapshot(removed);
    const newer = withSnapshot(second, snapshot([], 3n), second.asked);
    const older = withSnapshot(newer, snapshot([location("aoi", true)], 2n), first.asked);
    expect(older.friends).toEqual({});
    // The late answer to the first snapshot is the one the removal guards against.
    const late = withSnapshot(removed, snapshot([location("aoi", true)], 2n), first.asked);
    expect(late.friends).toEqual({});
    expect(Object.keys(late.removed)).toEqual(["aoi"]);
  });

  it("takes back a friend accepted again after the server's count started over", () => {
    // Removed as number 5; the count is lost, and the new friendship's pin never arrives.
    const removed = withoutFriend(withLocation(EMPTY, location("aoi", true)), "aoi", 5n);
    const read = answer(removed, snapshot([location("aoi", true)], 1n));
    expect(Object.keys(read.friends)).toEqual(["aoi"]);
    expect(read.removed).toEqual({});
  });

  it("takes a friend back from a snapshot read after the removal, or from a new pin", () => {
    const removed = withoutFriend(withLocation(EMPTY, location("aoi", true)), "aoi", 3n);
    expect(Object.keys(answer(removed, snapshot([location("aoi", true)], 3n)).friends)).toEqual([
      "aoi",
    ]);
    const back = withLocation(removed, location("aoi", true, { minute: 5 }));
    expect(Object.keys(answer(back, snapshot([location("aoi", true)], 2n)).friends)).toEqual([
      "aoi",
    ]);
  });

  it("orders nothing by a removal the server did not number", () => {
    const removed = withoutFriend(withLocation(EMPTY, location("aoi", true)), "aoi");
    expect(removed.friends).toEqual({});
    expect(Object.keys(answer(removed, snapshot([location("aoi", true)])).friends)).toEqual([
      "aoi",
    ]);
  });

  it("keeps a friend who appeared after the snapshot was read, though it lacks them", () => {
    const asking = askedForSnapshot(answer(EMPTY, snapshot([location("ren", true)], 4n)));
    const accepted = withLocation(asking, location("aoi", true), 5n);
    const late = withSnapshot(accepted, snapshot([location("ren", true)], 4n), asking.asked);
    expect(Object.keys(late.friends).sort()).toEqual(["aoi", "ren"]);
    expect(late.online).toEqual({ aoi: true, ren: true });
  });

  it("drops a friend who appeared when the snapshot, read after that, lacks them", () => {
    const asking = askedForSnapshot(EMPTY);
    const accepted = withLocation(asking, location("aoi", true), 5n);
    const read = withSnapshot(accepted, snapshot([], 5n), asking.asked);
    expect(read.friends).toEqual({});
  });

  it("orders nothing by an appearance from before the snapshot was asked for", () => {
    // A number from before a count that started over cannot keep a friend.
    const accepted = withLocation(EMPTY, location("aoi", true), 9n);
    const read = answer(accepted, snapshot([], 0n));
    expect(read.friends).toEqual({});
    expect(read.appeared).toEqual({});
  });

  it("drops the answer to a snapshot asked for before the one already applied", () => {
    const first = askedForSnapshot(EMPTY);
    const second = askedForSnapshot(first);
    const newer = withSnapshot(second, snapshot([location("aoi", true)]), second.asked);
    const older = withSnapshot(newer, snapshot([]), first.asked);
    expect(Object.keys(older.friends)).toEqual(["aoi"]);
  });

  it("drops the answers to snapshots asked for before everything was forgotten", () => {
    const asking = askedForSnapshot(EMPTY);
    const forgotten = cleared(withLocation(asking, location("aoi", true), 5n));
    const late = withSnapshot(forgotten, snapshot([location("aoi", true)]), asking.asked);
    expect(late.friends).toEqual({});
    expect(late.appeared).toEqual({});
  });
});
