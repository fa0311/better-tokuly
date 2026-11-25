import { create, geo } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { fromSnapshot, withLocation, withoutFriend } from "./live";

const location = (userId: string, online: boolean) =>
  create(geo.FriendLocationSchema, { userId, online, position: { lat: 35.68, lng: 139.76 } });

describe("live locations", () => {
  it("indexes a snapshot by friend and keeps only who is online", () => {
    const live = fromSnapshot([location("aoi", true), location("ren", false)]);
    expect(Object.keys(live.friends)).toEqual(["aoi", "ren"]);
    expect(live.online).toEqual({ aoi: true });
  });

  it("takes a friend's new location and drops one who left", () => {
    const live = withLocation(fromSnapshot([location("aoi", true)]), location("aoi", false));
    expect(live.online.aoi).toBe(false);
    expect(withoutFriend(live.friends, "aoi")).toEqual({});
  });
});
