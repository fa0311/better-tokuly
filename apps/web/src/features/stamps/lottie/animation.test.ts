import { describe, expect, it } from "vitest";
import {
  applyToPoint,
  cubicBezierEase,
  ellipseShape,
  frameAt,
  gradientStops,
  isAnimation,
  multiply,
  numberAt,
  rectShape,
  type ShapeData,
  scaleOf,
  shapeAt,
  starShape,
  transformMatrix,
  trimShapes,
  vectorAt,
} from "./animation";

const close = (actual: number[], expected: number[]) => {
  expect(actual).toHaveLength(expected.length);
  for (const [index, value] of actual.entries()) expect(value).toBeCloseTo(expected[index]!, 3);
};

describe("easing", () => {
  it("is linear without handles and stays on the ends", () => {
    expect(cubicBezierEase(0, 0, 1, 1, 0.3)).toBeCloseTo(0.3);
    expect(cubicBezierEase(0.42, 0, 0.58, 1, 0)).toBe(0);
    expect(cubicBezierEase(0.42, 0, 0.58, 1, 1)).toBe(1);
  });

  it("follows the curve: ease-in-out is slow at both ends and symmetric", () => {
    expect(cubicBezierEase(0.42, 0, 0.58, 1, 0.5)).toBeCloseTo(0.5, 3);
    expect(cubicBezierEase(0.42, 0, 0.58, 1, 0.2)).toBeLessThan(0.2);
    expect(cubicBezierEase(0.42, 0, 0.58, 1, 0.8)).toBeGreaterThan(0.8);
  });
});

describe("animated properties", () => {
  it("reads plain values of any size", () => {
    expect(numberAt({ a: 0, k: 42 }, 10, 0)).toBe(42);
    expect(vectorAt({ a: 0, k: [10, 20] }, 10, [0, 0])).toEqual([10, 20]);
    expect(numberAt(undefined, 10, 7)).toBe(7);
  });

  it("interpolates between keyframes and rests on the outer ones", () => {
    const property = { a: 1, k: [{ t: 10, s: [0, 100] }, { t: 20, s: [50, 0] }, { t: 30 }] };
    expect(vectorAt(property, 0, [])).toEqual([0, 100]);
    close(vectorAt(property, 15, []), [25, 50]);
    expect(vectorAt(property, 20, [])).toEqual([50, 0]);
    // The last keyframe has no value of its own: the animation rests where the previous segment ended.
    expect(vectorAt(property, 99, [])).toEqual([50, 0]);
  });

  it("supports the older format with an explicit end value, and hold keyframes", () => {
    close(vectorAt({ k: [{ t: 0, s: [0], e: [10] }, { t: 10 }] }, 5, []), [5]);
    expect(vectorAt({ k: [{ t: 0, s: [0], e: [10] }, { t: 10 }] }, 20, [])).toEqual([10]);
    const held = {
      k: [
        { t: 0, s: [1], h: 1 },
        { t: 10, s: [2] },
      ],
    };
    expect(numberAt(held, 9, 0)).toBe(1);
    expect(numberAt(held, 10, 0)).toBe(2);
  });

  it("eases each dimension with the keyframe's handles", () => {
    const property = {
      k: [
        { t: 0, s: [0], o: { x: [0.42], y: [0] }, i: { x: [0.58], y: [1] } },
        { t: 10, s: [100] },
      ],
    };
    expect(numberAt(property, 2, 0)).toBeLessThan(20);
    expect(numberAt(property, 5, 0)).toBeCloseTo(50, 1);
  });

  it("moves along a motion path when the keyframe has spatial tangents", () => {
    const property = {
      k: [
        { t: 0, s: [0, 0], to: [0, -50], ti: [0, -50] },
        { t: 10, s: [100, 0] },
      ],
    };
    const middle = vectorAt(property, 5, []);
    expect(middle[0]).toBeCloseTo(50);
    // The path bulges upwards instead of going straight.
    expect(middle[1]).toBeLessThan(-30);
  });

  it("morphs a path between keyframes", () => {
    const square = (size: number): ShapeData => ({
      c: true,
      v: [
        [0, 0],
        [size, 0],
        [size, size],
        [0, size],
      ],
      i: [
        [0, 0],
        [0, 0],
        [0, 0],
        [0, 0],
      ],
      o: [
        [0, 0],
        [0, 0],
        [0, 0],
        [0, 0],
      ],
    });
    const property = {
      k: [
        { t: 0, s: [square(10)] },
        { t: 10, s: [square(30)] },
      ],
    };
    expect(shapeAt(property, 5)?.v).toEqual([
      [0, 0],
      [20, 0],
      [20, 20],
      [0, 20],
    ]);
    expect(shapeAt({ k: square(10) }, 5)?.v[2]).toEqual([10, 10]);
  });
});

describe("transforms", () => {
  it("combines anchor, scale, rotation and position in Lottie's order", () => {
    const matrix = transformMatrix(
      { a: { k: [10, 10] }, p: { k: [100, 50] }, s: { k: [200, 200] }, r: { k: 90 } },
      0,
    );
    // The anchor lands on the position; a point 5 to the right of it ends up 10 below (rotated a quarter turn, scaled ×2).
    close(applyToPoint(matrix, 10, 10), [100, 50]);
    close(applyToPoint(matrix, 15, 10), [100, 60]);
    expect(scaleOf(matrix)).toBeCloseTo(2);
  });

  it("reads split position dimensions and multiplies parent before child", () => {
    const child = transformMatrix({ p: { s: true, k: 0, x: { k: 5 }, y: { k: 7 } } }, 0);
    close(applyToPoint(child, 0, 0), [5, 7]);
    const parent = transformMatrix({ s: { k: [200, 200] } }, 0);
    close(applyToPoint(multiply(parent, child), 0, 0), [10, 14]);
  });
});

describe("shapes", () => {
  it("builds closed contours for ellipses, rectangles and stars", () => {
    const ellipse = ellipseShape([0, 0], [20, 10]);
    expect(ellipse.v).toEqual([
      [0, -5],
      [10, 0],
      [0, 5],
      [-10, 0],
    ]);
    expect(rectShape([0, 0], [20, 10], 0).v).toHaveLength(4);
    expect(rectShape([0, 0], [20, 10], 3).v).toHaveLength(8);
    // Roundness can never exceed half the shorter side.
    expect(rectShape([0, 0], [20, 10], 99).v[0]).toEqual([10, 0]);
    expect(starShape([0, 0], 5, 10, 5, 0).v).toHaveLength(10);
    const triangle = starShape([0, 0], 3, 10, undefined, 0);
    expect(triangle.v).toHaveLength(3);
    close(triangle.v[0]!, [0, -10]);
  });
});

describe("trim paths", () => {
  const line: ShapeData = {
    c: false,
    v: [
      [0, 0],
      [100, 0],
    ],
    i: [
      [0, 0],
      [0, 0],
    ],
    o: [
      [0, 0],
      [0, 0],
    ],
  };
  const ends = (shape: ShapeData) => [shape.v[0]![0]!, shape.v[shape.v.length - 1]![0]!];
  // Lengths along a curve are measured in steps: cuts land within a fraction of a pixel.
  const near = (actual: number[], expected: number[]) => {
    expect(actual).toHaveLength(expected.length);
    for (const [index, value] of actual.entries())
      expect(Math.abs(value - expected[index]!)).toBeLessThan(0.25);
  };

  it("keeps the stretch between start and end", () => {
    const [kept] = trimShapes([line], 0.25, 0.75, 0);
    near(ends(kept!), [25, 75]);
  });

  it("returns everything for the full range and nothing for an empty one", () => {
    expect(trimShapes([line], 0, 1, 0)).toEqual([line]);
    expect(trimShapes([line], 0.4, 0.4, 0)).toEqual([]);
  });

  it("wraps around when the offset pushes the range past the end", () => {
    const pieces = trimShapes([line], 0, 0.5, 0.75);
    expect(pieces).toHaveLength(2);
    near(ends(pieces[0]!), [75, 100]);
    near(ends(pieces[1]!), [0, 25]);
  });

  it("cuts across segments and treats several paths as one line when asked to", () => {
    const corner: ShapeData = {
      c: false,
      v: [
        [0, 0],
        [100, 0],
        [100, 100],
      ],
      i: [
        [0, 0],
        [0, 0],
        [0, 0],
      ],
      o: [
        [0, 0],
        [0, 0],
        [0, 0],
      ],
    };
    const [kept] = trimShapes([corner], 0.25, 0.75, 0);
    expect(kept!.v).toHaveLength(3);
    near(kept!.v[0]!, [50, 0]);
    near(kept!.v[2]!, [100, 50]);

    const second: ShapeData = {
      ...line,
      v: [
        [0, 10],
        [100, 10],
      ],
    };
    // Half of two equal lines: all of the first, none of the second.
    const sequential = trimShapes([line, second], 0, 0.5, 0, true);
    expect(sequential).toHaveLength(1);
    near(ends(sequential[0]!), [0, 100]);
    // Each on its own: half of both.
    expect(trimShapes([line, second], 0, 0.5, 0)).toHaveLength(2);
  });
});

describe("gradients", () => {
  it("reads colour stops, and merges alpha stops into them", () => {
    expect(gradientStops([0, 1, 0, 0, 1, 0, 0, 1], 2)).toEqual([
      { offset: 0, color: [255, 0, 0, 1] },
      { offset: 1, color: [0, 0, 255, 1] },
    ]);
    const faded = gradientStops([0, 1, 0, 0, 1, 0, 0, 1, 0, 1, 0.5, 0, 1, 0], 2);
    expect(faded.map((stop) => stop.offset)).toEqual([0, 0.5, 1]);
    expect(faded[1]?.color).toEqual([127.5, 0, 127.5, 0]);
    expect(faded[2]?.color[3]).toBe(0);
  });
});

describe("playback", () => {
  it("loops over the animation's frames at its frame rate", () => {
    const animation = { fr: 30, ip: 0, op: 60 };
    expect(frameAt(animation, 0)).toBe(0);
    expect(frameAt(animation, 1000)).toBeCloseTo(30);
    expect(frameAt(animation, 2500)).toBeCloseTo(15);
    expect(frameAt({ fr: 30, ip: 10, op: 10 }, 500)).toBe(10);
  });

  it("recognises a Lottie animation", () => {
    expect(isAnimation({ fr: 30, ip: 0, op: 60, w: 100, h: 100, layers: [] })).toBe(true);
    expect(isAnimation({ animations: [] })).toBe(false);
    expect(isAnimation(null)).toBe(false);
  });
});
