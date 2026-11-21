import { describe, expect, it } from "vitest";
import {
  addOutline,
  applyMask,
  type Bitmap,
  contentBounds,
  cornerSeeds,
  distanceToShape,
  fitPlacement,
  fitWithin,
  floodErase,
  fullMask,
  History,
  outsideOfShape,
  paintMask,
  toPicture,
  zoomPlacement,
} from "./image-ops";

type Color = [number, number, number, number];
const WHITE: Color = [255, 255, 255, 255];
const RED: Color = [220, 30, 40, 255];
const CLEAR: Color = [0, 0, 0, 0];

/** A picture filled with `background`, with a `subject` rectangle on it. */
function picture(width: number, height: number, background: Color, subject?: { x: number; y: number; width: number; height: number; color: Color }): Bitmap {
  const data = new Uint8ClampedArray(width * height * 4);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const inside = subject && x >= subject.x && x < subject.x + subject.width && y >= subject.y && y < subject.y + subject.height;
      data.set(inside ? subject.color : background, (y * width + x) * 4);
    }
  }
  return { width, height, data };
}

const at = (mask: Uint8Array, width: number, x: number, y: number) => mask[y * width + x];

describe("magic removal", () => {
  it("erases a uniform background from the corners and keeps the subject", () => {
    const source = picture(20, 20, WHITE, { x: 6, y: 6, width: 8, height: 8, color: RED });
    const mask = fullMask(20, 20);
    const erased = floodErase(source, mask, cornerSeeds(20, 20), 24);

    expect(erased).toBe(20 * 20 - 8 * 8);
    expect(at(mask, 20, 0, 0)).toBe(0);
    expect(at(mask, 20, 19, 19)).toBe(0);
    expect(at(mask, 20, 5, 10)).toBe(0);
    expect(at(mask, 20, 6, 6)).toBe(255);
    expect(at(mask, 20, 13, 13)).toBe(255);
  });

  it("does not reach background enclosed by the subject", () => {
    // A red ring with a white hole in the middle.
    const source = picture(20, 20, WHITE, { x: 4, y: 4, width: 12, height: 12, color: RED });
    for (let y = 8; y < 12; y++) for (let x = 8; x < 12; x++) source.data.set(WHITE, (y * 20 + x) * 4);
    const mask = fullMask(20, 20);
    floodErase(source, mask, cornerSeeds(20, 20), 24);

    expect(at(mask, 20, 1, 1)).toBe(0);
    expect(at(mask, 20, 9, 9)).toBe(255);
    // Tapping the hole removes it as well.
    floodErase(source, mask, [{ x: 9, y: 9 }], 24);
    expect(at(mask, 20, 9, 9)).toBe(0);
    expect(at(mask, 20, 5, 5)).toBe(255);
  });

  it("follows the tolerance: near-white is background only when it is allowed to be", () => {
    const offWhite: Color = [235, 235, 235, 255];
    const source = picture(10, 10, WHITE, { x: 0, y: 5, width: 10, height: 5, color: offWhite });

    const strict = fullMask(10, 10);
    floodErase(source, strict, [{ x: 0, y: 0 }], 4);
    expect(at(strict, 10, 5, 2)).toBe(0);
    expect(at(strict, 10, 5, 8)).toBe(255);

    const loose = fullMask(10, 10);
    floodErase(source, loose, [{ x: 0, y: 0 }], 40);
    expect(at(loose, 10, 5, 8)).toBe(0);
  });

  it("fades the pixels just past the tolerance instead of cutting hard", () => {
    const edge: Color = [229, 229, 229, 255];
    const source = picture(9, 1, WHITE, { x: 4, y: 0, width: 1, height: 1, color: edge });
    source.data.set(RED, 5 * 4);
    source.data.set(RED, 6 * 4);
    const mask = fullMask(9, 1);
    floodErase(source, mask, [{ x: 0, y: 0 }], 20);

    expect(at(mask, 9, 3, 0)).toBe(0);
    const faded = at(mask, 9, 4, 0)!;
    expect(faded).toBeGreaterThan(0);
    expect(faded).toBeLessThan(255);
    expect(at(mask, 9, 5, 0)).toBe(255);
  });

  it("treats the empty area of an already transparent picture as its background", () => {
    const source = picture(12, 12, CLEAR, { x: 4, y: 4, width: 4, height: 4, color: RED });
    const mask = fullMask(12, 12);
    floodErase(source, mask, cornerSeeds(12, 12), 24);
    expect(at(mask, 12, 0, 0)).toBe(0);
    expect(at(mask, 12, 5, 5)).toBe(255);
  });

  it("fills from each corner by that corner's own colour", () => {
    // Left half white, right half black: two different backgrounds.
    const source = picture(12, 6, WHITE, { x: 6, y: 0, width: 6, height: 6, color: [0, 0, 0, 255] });
    source.data.set(RED, (3 * 12 + 2) * 4);
    const mask = fullMask(12, 6);
    floodErase(source, mask, cornerSeeds(12, 6), 10);
    expect(at(mask, 12, 0, 0)).toBe(0);
    expect(at(mask, 12, 11, 5)).toBe(0);
    expect(at(mask, 12, 2, 3)).toBe(255);
  });
});

describe("brush", () => {
  it("erases along the stroke and restores what it erased", () => {
    const mask = fullMask(40, 40);
    const touched = paintMask(mask, 40, 40, { x: 10, y: 20 }, { x: 30, y: 20 }, 4, "erase");

    expect(at(mask, 40, 20, 20)).toBe(0);
    expect(at(mask, 40, 10, 20)).toBe(0);
    expect(at(mask, 40, 20, 30)).toBe(255);
    expect(touched).toEqual({ x: 5, y: 15, width: 31, height: 11 });

    paintMask(mask, 40, 40, { x: 20, y: 20 }, { x: 20, y: 20 }, 3, "restore");
    expect(at(mask, 40, 20, 20)).toBe(255);
    expect(at(mask, 40, 28, 20)).toBe(0);
  });

  it("has a soft rim and stays inside the picture", () => {
    const mask = fullMask(20, 20);
    paintMask(mask, 20, 20, { x: 0, y: 0 }, { x: 0, y: 0 }, 10, "erase");
    expect(at(mask, 20, 0, 0)).toBe(0);
    const rim = at(mask, 20, 9, 0)!;
    expect(rim).toBeGreaterThan(0);
    expect(rim).toBeLessThan(255);
    expect(at(mask, 20, 12, 0)).toBe(255);
  });
});

describe("cut-out", () => {
  it("combines the picture with the mask", () => {
    const source = picture(2, 1, RED);
    const out = new Uint8ClampedArray(8);
    applyMask(source, new Uint8Array([255, 0]), out);
    expect([...out]).toEqual([220, 30, 40, 255, 220, 30, 40, 0]);
  });

  it("can redraw just the touched area", () => {
    const source = picture(4, 4, RED);
    const out = new Uint8ClampedArray(4 * 4 * 4);
    applyMask(source, fullMask(4, 4), out, { x: 1, y: 1, width: 2, height: 2 });
    const alpha = (x: number, y: number) => out[(y * 4 + x) * 4 + 3];
    expect(alpha(1, 1)).toBe(255);
    expect(alpha(2, 2)).toBe(255);
    expect(alpha(0, 0)).toBe(0);
    expect(alpha(3, 2)).toBe(0);
  });

  it("finds the bounds of what is left", () => {
    const source = picture(20, 20, WHITE, { x: 6, y: 7, width: 8, height: 5, color: RED });
    const mask = fullMask(20, 20);
    floodErase(source, mask, cornerSeeds(20, 20), 24);
    expect(contentBounds(source, mask)).toEqual({ x: 6, y: 7, width: 8, height: 5 });
    expect(contentBounds(source, new Uint8Array(400))).toBeNull();
    expect(contentBounds(source)).toEqual({ x: 0, y: 0, width: 20, height: 20 });
  });
});

describe("framing", () => {
  it("limits the working size without changing proportions", () => {
    expect(fitWithin(4000, 3000, 1024)).toEqual({ width: 1024, height: 768 });
    expect(fitWithin(300, 200, 1024)).toEqual({ width: 300, height: 200 });
  });

  it("centres the content in the frame, inside the margin", () => {
    const placement = fitPlacement({ x: 100, y: 50, width: 200, height: 100 }, 512, 56);
    expect(placement.scale).toBe(2);
    // The content spans 56..456 horizontally and is centred vertically.
    expect(placement.x + 100 * placement.scale).toBe(56);
    expect(placement.y + 50 * placement.scale).toBe(156);
  });

  it("zooms around a point and converts frame points back to the picture", () => {
    const placement = { scale: 1, x: 0, y: 0 };
    const zoomed = zoomPlacement(placement, 2, { x: 100, y: 100 }, 0.1, 8);
    expect(zoomed).toEqual({ scale: 2, x: -100, y: -100 });
    expect(toPicture(zoomed, { x: 100, y: 100 })).toEqual({ x: 100, y: 100 });
    expect(zoomPlacement(placement, 100, { x: 0, y: 0 }, 0.1, 8).scale).toBe(8);
  });
});

describe("sticker outline", () => {
  it("measures the distance to the shape exactly", () => {
    const shape = picture(7, 7, CLEAR, { x: 3, y: 3, width: 1, height: 1, color: RED });
    const distances = distanceToShape(shape);
    expect(distances[3 * 7 + 3]).toBe(0);
    expect(distances[3 * 7 + 5]).toBe(4);
    expect(distances[0]).toBe(18);
  });

  it("paints a white rim around the subject and leaves the subject alone", () => {
    const bitmap = picture(24, 24, CLEAR, { x: 8, y: 8, width: 8, height: 8, color: RED });
    addOutline(bitmap, 3);
    const pixel = (x: number, y: number) => [...bitmap.data.slice((y * 24 + x) * 4, (y * 24 + x) * 4 + 4)];

    expect(pixel(10, 10)).toEqual(RED);
    expect(pixel(6, 10)).toEqual(WHITE);
    expect(pixel(10, 17)).toEqual(WHITE);
    expect(pixel(1, 1)[3]).toBe(0);
    // The outline grows the visible area by its thickness on every side.
    expect(contentBounds(bitmap)).toEqual({ x: 5, y: 5, width: 14, height: 14 });
  });

  it("leaves holes inside the subject open", () => {
    // A red ring: the hole in its middle was erased and must stay see-through.
    const bitmap = picture(30, 30, CLEAR, { x: 8, y: 8, width: 14, height: 14, color: RED });
    for (let y = 13; y < 17; y++) for (let x = 13; x < 17; x++) bitmap.data.set(CLEAR, (y * 30 + x) * 4);
    const outside = outsideOfShape(bitmap);
    expect(outside[0]).toBe(1);
    expect(outside[15 * 30 + 15]).toBe(0);
    expect(outside[10 * 30 + 10]).toBe(0);

    addOutline(bitmap, 3);
    const alpha = (x: number, y: number) => bitmap.data[(y * 30 + x) * 4 + 3];
    expect(alpha(15, 15)).toBe(0);
    expect(alpha(6, 15)).toBe(255);
  });

  it("puts the subject's own soft edge onto the outline", () => {
    const bitmap = picture(12, 12, CLEAR, { x: 4, y: 4, width: 4, height: 4, color: RED });
    // A half-transparent edge pixel, as anti-aliasing leaves it.
    bitmap.data.set([220, 30, 40, 140], (5 * 12 + 4) * 4);
    addOutline(bitmap, 2);
    const edge = [...bitmap.data.slice((5 * 12 + 4) * 4, (5 * 12 + 4) * 4 + 4)];
    expect(edge[3]).toBe(255);
    // Red over white: lighter than the subject, not see-through any more.
    expect(edge[1]).toBeGreaterThan(100);
  });

  it("does nothing without a thickness", () => {
    const bitmap = picture(8, 8, CLEAR, { x: 3, y: 3, width: 2, height: 2, color: RED });
    const before = [...bitmap.data];
    addOutline(bitmap, 0);
    expect([...bitmap.data]).toEqual(before);
  });
});

describe("history", () => {
  it("undoes and redoes, and forgets the redo branch after a new step", () => {
    const history = new History("start");
    expect(history.canUndo).toBe(false);
    history.push("a");
    history.push("b");

    expect(history.undo()).toBe("a");
    expect(history.undo()).toBe("start");
    expect(history.undo()).toBeUndefined();
    expect(history.redo()).toBe("a");
    expect(history.canRedo).toBe(true);

    history.push("c");
    expect(history.canRedo).toBe(false);
    expect(history.undo()).toBe("a");
  });

  it("amends the latest step in place", () => {
    const history = new History("start");
    history.push("draft");
    history.amend("final");
    expect(history.undo()).toBe("start");
    expect(history.redo()).toBe("final");
  });

  it("keeps a bounded number of steps", () => {
    const history = new History(0, 3);
    for (let step = 1; step <= 10; step++) history.push(step);
    expect(history.undo()).toBe(9);
    expect(history.undo()).toBe(8);
    expect(history.undo()).toBe(7);
    expect(history.undo()).toBeUndefined();
  });
});
