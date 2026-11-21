import { describe, expect, it } from "vitest";
import { clampView, visibleRegion } from "./crop";

const LANDSCAPE = { width: 4000, height: 2000 };

describe("visibleRegion", () => {
  it("covers the largest centered square of a landscape image at zoom 1", () => {
    expect(visibleRegion({ centerX: 0.5, centerY: 0.5, zoom: 1 }, LANDSCAPE, 1)).toEqual({ x: 1000, y: 0, width: 2000, height: 2000 });
  });

  it("uses the full width for a frame wider than the image", () => {
    const region = visibleRegion({ centerX: 0.5, centerY: 0.5, zoom: 1 }, LANDSCAPE, 3);
    expect(region.width).toBe(4000);
    expect(region.height).toBeCloseTo(4000 / 3);
  });

  it("shrinks with the zoom", () => {
    expect(visibleRegion({ centerX: 0.5, centerY: 0.5, zoom: 2 }, LANDSCAPE, 1)).toEqual({ x: 1500, y: 500, width: 1000, height: 1000 });
  });
});

describe("clampView", () => {
  it("keeps the frame inside the image when dragged past an edge", () => {
    const view = clampView({ centerX: -1, centerY: 2, zoom: 2 }, LANDSCAPE, 1, 4);
    const region = visibleRegion(view, LANDSCAPE, 1);
    expect(region.x).toBeCloseTo(0);
    expect(region.y + region.height).toBeCloseTo(LANDSCAPE.height);
  });

  it("limits the zoom to its range", () => {
    expect(clampView({ centerX: 0.5, centerY: 0.5, zoom: 0.2 }, LANDSCAPE, 1, 4).zoom).toBe(1);
    expect(clampView({ centerX: 0.5, centerY: 0.5, zoom: 9 }, LANDSCAPE, 1, 4).zoom).toBe(4);
  });

  it("pins the axis that has no room to move at zoom 1", () => {
    expect(clampView({ centerX: 0.5, centerY: 0.1, zoom: 1 }, LANDSCAPE, 1, 4).centerY).toBe(0.5);
  });
});
