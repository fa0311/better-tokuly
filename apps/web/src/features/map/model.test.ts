import { create, fromDate, geo, type MessageInitShape } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  clusterOnScreen,
  decodePolyline,
  fanOffset,
  ghostRuleFor,
  ghostSummary,
  pinState,
  seeEachOther,
  stayMillis,
} from "./model";

const NOW = Date.UTC(2026, 10, 22, 9, 0, 0);
const at = (minutesAgo: number) => fromDate(new Date(NOW - minutesAgo * 60_000));
const pin = (init: MessageInitShape<typeof geo.FriendLocationSchema>) =>
  create(geo.FriendLocationSchema, {
    position: { lat: 35.66, lng: 139.7 },
    precision: geo.GhostMode.PRECISE,
    ...(init as object),
  });

describe("pinState", () => {
  it("is live for a fresh precise pin and stale after ten minutes", () => {
    expect(pinState(pin({ recordedAt: at(1) }), NOW)).toBe("live");
    expect(pinState(pin({ recordedAt: at(11) }), NOW)).toBe("stale");
  });
  it("gives a blurred pin longer, because its time is coarse", () => {
    expect(pinState(pin({ recordedAt: at(19), precision: geo.GhostMode.BLURRED }), NOW)).toBe(
      "blurred",
    );
    expect(pinState(pin({ recordedAt: at(45), precision: geo.GhostMode.BLURRED }), NOW)).toBe(
      "stale",
    );
  });
  it("is paused when the device said it stopped sharing, whatever the time", () => {
    expect(pinState(pin({ recordedAt: at(0), sharingPaused: true }), NOW)).toBe("paused");
  });
});

describe("stayMillis", () => {
  it("shows time on the spot from five minutes, and never while moving", () => {
    expect(stayMillis(pin({ recordedAt: at(0), stationarySince: at(120) }), NOW)).toBe(
      120 * 60_000,
    );
    expect(stayMillis(pin({ recordedAt: at(0), stationarySince: at(2) }), NOW)).toBeNull();
    expect(
      stayMillis(pin({ recordedAt: at(0), stationarySince: at(120), speedMps: 3 }), NOW),
    ).toBeNull();
  });
});

describe("ghost rules", () => {
  const settings = create(geo.GetGhostSettingsResponseSchema, {
    everyone: { mode: geo.GhostMode.PRECISE },
    overrides: {
      ren: { mode: geo.GhostMode.FROZEN, until: at(-30) },
      mio: { mode: geo.GhostMode.BLURRED, until: at(5) },
    },
  });
  it("uses an override while it runs and the default once it ended", () => {
    expect(ghostRuleFor(settings, "ren", NOW)).toBe(geo.GhostMode.FROZEN);
    expect(ghostRuleFor(settings, "mio", NOW)).toBe(geo.GhostMode.PRECISE);
    expect(ghostRuleFor(settings, "aoi", NOW)).toBe(geo.GhostMode.PRECISE);
  });
  it("summarizes who is hidden", () => {
    expect(ghostSummary(settings, NOW)).toMatchObject({
      mode: geo.GhostMode.FROZEN,
      scope: "some",
      hiddenCount: 1,
    });
    expect(
      ghostSummary(
        create(geo.GetGhostSettingsResponseSchema, { everyone: { mode: geo.GhostMode.PRECISE } }),
        NOW,
      ),
    ).toBeNull();
    expect(
      ghostSummary(
        create(geo.GetGhostSettingsResponseSchema, { everyone: { mode: geo.GhostMode.BLURRED } }),
        NOW,
      ),
    ).toMatchObject({ scope: "everyone" });
  });
  it("only counts a pair as seeing each other when both sides are precise and the pin is fresh", () => {
    expect(seeEachOther(pin({ recordedAt: at(1) }), geo.GhostMode.PRECISE, NOW)).toBe(true);
    expect(seeEachOther(pin({ recordedAt: at(1) }), geo.GhostMode.BLURRED, NOW)).toBe(false);
    expect(
      seeEachOther(
        pin({ recordedAt: at(1), precision: geo.GhostMode.BLURRED }),
        geo.GhostMode.PRECISE,
        NOW,
      ),
    ).toBe(false);
    expect(seeEachOther(pin({ recordedAt: at(60) }), geo.GhostMode.PRECISE, NOW)).toBe(false);
  });
});

describe("clusterOnScreen", () => {
  it("groups overlapping pins transitively and leaves loners alone", () => {
    const groups = clusterOnScreen(
      [
        { id: "c", x: 60, y: 0 },
        { id: "a", x: 0, y: 0 },
        { id: "b", x: 30, y: 0 },
        { id: "far", x: 500, y: 500 },
      ],
      40,
    );
    expect(groups).toEqual([["a", "b", "c"]]);
  });
});

describe("fanOffset", () => {
  it("spreads members on a ring wide enough not to overlap", () => {
    const offsets = Array.from({ length: 6 }, (_, i) => fanOffset(i, 6));
    expect(offsets[0]).toEqual([0, -59]);
    for (let i = 1; i < offsets.length; i++) {
      const [ax, ay] = offsets[i - 1] as [number, number];
      const [bx, by] = offsets[i] as [number, number];
      expect(Math.hypot(ax - bx, ay - by)).toBeGreaterThan(50);
    }
  });
});

describe("decodePolyline", () => {
  it("decodes the reference example", () => {
    expect(decodePolyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@")).toEqual([
      [-120.2, 38.5],
      [-120.95, 40.7],
      [-126.453, 43.252],
    ]);
  });
});
