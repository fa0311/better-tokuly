import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, geo } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { type LiveLocations, withLocation, withoutFriend, withSnapshot } from "./live";

const EMPTY: LiveLocations = { friends: {}, online: {} };

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

describe("live locations", () => {
  it("indexes a snapshot by friend and keeps only who is online", () => {
    const live = withSnapshot(EMPTY, [location("aoi", true), location("ren", false)]);
    expect(Object.keys(live.friends)).toEqual(["aoi", "ren"]);
    expect(live.online).toEqual({ aoi: true });
  });

  it("takes a friend's new location and drops one who left", () => {
    const live = withLocation(withSnapshot(EMPTY, [location("aoi", true)]), location("aoi", false));
    expect(live.online.aoi).toBe(false);
    expect(withoutFriend(live.friends, "aoi")).toEqual({});
  });

  it("keeps a pushed pin newer than a snapshot that arrives after it", () => {
    const pushed = withLocation(EMPTY, location("aoi", true, { minute: 5, lat: 35.7 }));
    const live = withSnapshot(pushed, [
      location("aoi", false, { minute: 4 }),
      location("ren", true, { minute: 4 }),
    ]);
    expect(live.friends.aoi?.position?.lat).toBe(35.7);
    expect(live.online).toEqual({ aoi: true, ren: true });
  });

  it("takes the snapshot's pin when it is the newer one, and drops who is not in it", () => {
    const held = withLocation(EMPTY, location("aoi", true, { minute: 4 }));
    const live = withSnapshot(withLocation(held, location("ren", true)), [
      location("aoi", false, { minute: 5, lat: 35.7 }),
    ]);
    expect(live.friends.aoi?.position?.lat).toBe(35.7);
    expect(live.online).toEqual({});
    expect(Object.keys(live.friends)).toEqual(["aoi"]);
  });

  it("keeps a blurred pin against a precise one, whose time is not rounded", () => {
    const blurred = location("aoi", true, { minute: 0, precision: geo.GhostMode.BLURRED });
    const live = withSnapshot(withLocation(EMPTY, blurred), [location("aoi", true, { minute: 3 })]);
    expect(live.friends.aoi?.precision).toBe(geo.GhostMode.BLURRED);
  });
});
