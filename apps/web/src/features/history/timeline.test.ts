import { create, fromDate, geo } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { buildTimeline, decodePolyline, positionAt, type TrackPoint } from "./timeline";

const T0 = Date.UTC(2026, 10, 22, 8, 0, 0);
const minutes = (count: number) => T0 + count * 60_000;

function visit(id: string, lng: number, arrive: number, depart?: number) {
  return create(geo.VisitSchema, {
    id,
    name: id,
    center: { lat: 35.6595, lng },
    arrivedAt: fromDate(new Date(arrive)),
    departedAt: depart ? fromDate(new Date(depart)) : undefined,
    durationSeconds: BigInt(Math.round(((depart ?? arrive) - arrive) / 1000)),
  });
}

/** A walk east along one latitude: 14 m every 10 s. */
function walk(
  fromLng: number,
  start: number,
  steps: number,
  activity = geo.Activity.WALKING,
): TrackPoint[] {
  const metersPerDegree = 111_320 * Math.cos((35.6595 * Math.PI) / 180);
  return Array.from({ length: steps + 1 }, (_, index) => ({
    lat: 35.6595,
    lng: fromLng + (index * 14) / metersPerDegree,
    at: start + index * 10_000,
    activity,
  }));
}

describe("decodePolyline", () => {
  it("decodes the reference example", () => {
    expect(decodePolyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@")).toEqual([
      { lat: 38.5, lng: -120.2 },
      { lat: 40.7, lng: -120.95 },
      { lat: 43.252, lng: -126.453 },
    ]);
  });

  it("returns nothing for an empty or cut-off string", () => {
    expect(decodePolyline("")).toEqual([]);
    expect(decodePolyline("_p~iF")).toEqual([]);
  });
});

describe("buildTimeline", () => {
  const a = visit("a", 139.7005, minutes(0), minutes(9));
  const b = visit("b", 139.7098, minutes(19));

  it("puts the way between two visits, measured from the route", () => {
    const items = buildTimeline([b, a], walk(139.7005, minutes(9), 60));
    expect(items.map((item) => item.kind)).toEqual(["visit", "move", "visit"]);
    const move = items[1]!;
    if (move.kind !== "move") throw new Error("expected a move");
    expect(move.activity).toBe(geo.Activity.WALKING);
    expect(move.distanceMeters).toBeGreaterThan(800);
    expect(move.distanceMeters).toBeLessThan(880);
    expect(move.to - move.from).toBe(10 * 60_000);
  });

  it("guesses the way of travelling from the speed when the device did not say", () => {
    const fast = walk(139.7005, minutes(9), 60, geo.Activity.UNSPECIFIED).map((point, index) => ({
      ...point,
      at: minutes(9) + index * 1000,
    }));
    const quick = buildTimeline([a, visit("b", 139.7098, minutes(10) + 30_000)], fast)[1]!;
    expect(quick.kind === "move" && quick.activity).toBe(geo.Activity.DRIVING);
  });

  it("still shows that the person moved when there is no route", () => {
    const move = buildTimeline([a, b], [])[1]!;
    if (move.kind !== "move") throw new Error("expected a move");
    expect(move.activity).toBe(geo.Activity.UNSPECIFIED);
    expect(move.distanceMeters).toBeGreaterThan(800);
  });

  it("skips a hop too short to mention", () => {
    const next = visit("b", 139.70051, minutes(9) + 20_000);
    expect(buildTimeline([a, next], []).map((item) => item.kind)).toEqual(["visit", "visit"]);
  });
});

describe("positionAt", () => {
  const track = walk(139.7005, minutes(0), 6);

  it("clamps to the ends", () => {
    expect(positionAt(track, minutes(-5))).toMatchObject({ index: 0, lng: 139.7005 });
    expect(positionAt(track, minutes(5))).toMatchObject({ index: 6 });
    expect(positionAt([], minutes(0))).toBeNull();
  });

  it("interpolates between two points", () => {
    const at = positionAt(track, minutes(0) + 15_000)!;
    expect(at.index).toBe(1);
    expect(at.lng).toBeCloseTo((track[1]!.lng + track[2]!.lng) / 2, 9);
  });
});
