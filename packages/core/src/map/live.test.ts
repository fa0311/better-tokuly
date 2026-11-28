import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, geo } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { NO_LOCATIONS as EMPTY, withLocation, withoutFriend, withSnapshot } from "./live";

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

/** A snapshot read when the viewer had been sent `removalSequence` removals. */
const snapshot = (locations: geo.FriendLocation[], removalSequence = 0n) => ({
  locations,
  removalSequence,
});

describe("live locations", () => {
  it("indexes a snapshot by friend and keeps only who is online", () => {
    const live = withSnapshot(EMPTY, snapshot([location("aoi", true), location("ren", false)]));
    expect(Object.keys(live.friends)).toEqual(["aoi", "ren"]);
    expect(live.online).toEqual({ aoi: true });
  });

  it("takes a friend's new location and drops one who left", () => {
    const live = withLocation(
      withSnapshot(EMPTY, snapshot([location("aoi", true)])),
      location("aoi", false),
    );
    expect(live.online.aoi).toBe(false);
    expect(withoutFriend(live, "aoi").friends).toEqual({});
  });

  it("keeps a pushed pin newer than a snapshot that arrives after it", () => {
    const pushed = withLocation(EMPTY, location("aoi", true, { minute: 5, lat: 35.7 }));
    const live = withSnapshot(
      pushed,
      snapshot([location("aoi", false, { minute: 4 }), location("ren", true, { minute: 4 })]),
    );
    expect(live.friends.aoi?.position?.lat).toBe(35.7);
    expect(live.online).toEqual({ aoi: true, ren: true });
  });

  it("takes the snapshot's pin when it is the newer one, and drops who is not in it", () => {
    const held = withLocation(EMPTY, location("aoi", true, { minute: 4 }));
    const live = withSnapshot(
      withLocation(held, location("ren", true)),
      snapshot([location("aoi", false, { minute: 5, lat: 35.7 })]),
    );
    expect(live.friends.aoi?.position?.lat).toBe(35.7);
    expect(live.online).toEqual({});
    expect(Object.keys(live.friends)).toEqual(["aoi"]);
  });

  it("keeps a blurred pin against a precise one, whose time is not rounded", () => {
    const blurred = location("aoi", true, { minute: 0, precision: geo.GhostMode.BLURRED });
    const live = withSnapshot(
      withLocation(EMPTY, blurred),
      snapshot([location("aoi", true, { minute: 3 })]),
    );
    expect(live.friends.aoi?.precision).toBe(geo.GhostMode.BLURRED);
  });

  it("keeps a friend removed after the snapshot was read off the map", () => {
    const held = withSnapshot(EMPTY, snapshot([location("aoi", true), location("ren", true)], 2n));
    const removed = withoutFriend(held, "aoi", 3n);
    const late = withSnapshot(
      removed,
      snapshot([location("aoi", true), location("ren", true)], 2n),
    );
    expect(Object.keys(late.friends)).toEqual(["ren"]);
    expect(late.online).toEqual({ ren: true });
  });

  it("takes a friend back from a snapshot read after the removal, or from a new pin", () => {
    const removed = withoutFriend(withLocation(EMPTY, location("aoi", true)), "aoi", 3n);
    expect(
      Object.keys(withSnapshot(removed, snapshot([location("aoi", true)], 3n)).friends),
    ).toEqual(["aoi"]);
    const back = withLocation(removed, location("aoi", true, { minute: 5 }));
    expect(Object.keys(withSnapshot(back, snapshot([location("aoi", true)], 2n)).friends)).toEqual([
      "aoi",
    ]);
  });

  it("orders nothing by a removal the server did not number", () => {
    const removed = withoutFriend(withLocation(EMPTY, location("aoi", true)), "aoi");
    expect(removed.friends).toEqual({});
    expect(Object.keys(withSnapshot(removed, snapshot([location("aoi", true)])).friends)).toEqual([
      "aoi",
    ]);
  });
});
