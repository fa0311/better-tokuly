import { create, geo } from "@tokuly/api";
import { boundsOf, CLOSEST_ZOOM, crowdsAt } from "../crowds";

const pin = (userId: string, lat: number, lng: number) =>
  create(geo.FriendLocationSchema, { userId, position: { lat, lng } });

// Shibuya station, a café 150 m away, and Yokohama.
const pins = [pin("a", 35.658, 139.7016), pin("b", 35.6592, 139.7024), pin("c", 35.4658, 139.6223)];

describe("crowdsAt", () => {
  it("stacks the pins that would cover each other, and only those", () => {
    const crowds = crowdsAt(pins, 11);
    expect(crowds.map((crowd) => crowd.key)).toEqual(["a,b"]);
    expect(crowds[0]?.lat).toBeCloseTo(35.6586, 4);
    expect(boundsOf(crowds[0]!)).toEqual([139.7016, 35.658, 139.7024, 35.6592]);
  });

  it("parts them once the map is zoomed in far enough", () => {
    expect(crowdsAt(pins, CLOSEST_ZOOM)).toEqual([]);
  });

  it("stacks everyone when the whole region fits a thumb", () => {
    expect(crowdsAt(pins, 5).map((crowd) => crowd.key)).toEqual(["a,b,c"]);
  });

  it("leaves the friend whose card is open out of the stack", () => {
    expect(crowdsAt(pins, 11, "a")).toEqual([]);
  });

  it("keeps friends in the very same place together at any zoom", () => {
    const same = [pin("a", 35.658, 139.7016), pin("b", 35.658, 139.7016)];
    expect(crowdsAt(same, CLOSEST_ZOOM).map((crowd) => crowd.key)).toEqual(["a,b"]);
  });
});
