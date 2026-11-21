/**
 * The data side of the Lottie player: the parts of the Lottie JSON format it
 * understands, animated property evaluation, 2D matrices and path geometry.
 * Everything here is pure; the drawing is in `render.ts`.
 */

// ───────────────────────────── Format ─────────────────────────────

/** Bezier handles of a keyframe's easing curve; per dimension when arrays. */
interface Handle {
  x: number | number[];
  y: number | number[];
}

export interface Keyframe {
  /** Frame at which this keyframe sits. */
  t: number;
  /** Value at this keyframe. */
  s?: unknown;
  /** Value at the end of the segment (older exports; newer ones use the next keyframe's `s`). */
  e?: unknown;
  /** 1 = hold: the value jumps at the next keyframe. */
  h?: number;
  i?: Handle;
  o?: Handle;
  /** Spatial tangents of a motion path. */
  to?: number[];
  ti?: number[];
}

/** A value that may change over time. `k` is the value, or keyframes. */
export interface Property {
  a?: number;
  k: unknown;
}

/** A bezier contour: vertices plus tangents relative to their vertex. */
export interface ShapeData {
  v: number[][];
  i: number[][];
  o: number[][];
  c?: boolean;
}

export interface Transform {
  a?: Property;
  p?: Property & { s?: boolean; x?: Property; y?: Property };
  s?: Property;
  r?: Property;
  o?: Property;
  sk?: Property;
  sa?: Property;
}

export interface ShapeItem extends Transform {
  ty: string;
  hd?: boolean;
  /** Group contents. */
  it?: ShapeItem[];
  /** Path geometry (`sh`). */
  ks?: Property;
  /** Colour (`fl`, `st`). */
  c?: Property;
  /** Stroke width. */
  w?: Property;
  lc?: number;
  lj?: number;
  ml?: number;
  /** Dashes of a stroke (on parametric shapes `d` is a drawing direction instead, which is ignored). */
  d?: { n: string; v: Property }[] | number;
  /** Gradient: colour stops, end point, type (1 linear, 2 radial). */
  g?: { p: number; k: Property };
  e?: Property;
  t?: number;
  /** Trim paths: mode (1 all paths at once, 2 one after another). */
  m?: number;
  /** Star: type (1 star, 2 polygon), points, radii. */
  sy?: number;
  pt?: Property;
  or?: Property;
  ir?: Property;
}

export interface Mask {
  mode?: string;
  inv?: boolean;
  pt?: Property;
  o?: Property;
}

export interface Layer {
  /** 0 precomposition, 1 solid, 2 image, 3 null, 4 shapes. */
  ty: number;
  ind?: number;
  parent?: number;
  ip: number;
  op: number;
  st?: number;
  sr?: number;
  hd?: boolean;
  ks?: Transform;
  /** This layer is another layer's matte. */
  td?: number;
  /** Matte type: 1 alpha, 2 inverted alpha, 3 luma, 4 inverted luma. */
  tt?: number;
  /** Index (`ind`) of the matte layer; the layer above when unset. */
  tp?: number;
  refId?: string;
  shapes?: ShapeItem[];
  w?: number;
  h?: number;
  sc?: string;
  sw?: number;
  sh?: number;
  masksProperties?: Mask[];
  /** Time remapping of a precomposition, in seconds. */
  tm?: Property;
  bm?: number;
}

export interface Asset {
  id: string;
  layers?: Layer[];
  w?: number;
  h?: number;
  /** Image: folder, file name, 1 when `p` is a data URL. */
  u?: string;
  p?: string;
  e?: number;
}

export interface Animation {
  fr: number;
  ip: number;
  op: number;
  w: number;
  h: number;
  layers: Layer[];
  assets?: Asset[];
}

/** Whether a parsed JSON value looks like a Lottie animation this player can run. */
export function isAnimation(value: unknown): value is Animation {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<Animation>;
  return Array.isArray(candidate.layers) && typeof candidate.w === "number" && typeof candidate.h === "number" && typeof candidate.op === "number";
}

// ───────────────────────────── Easing ─────────────────────────────

/**
 * The y of a CSS-style cubic bezier easing curve (0,0)-(x1,y1)-(x2,y2)-(1,1)
 * at horizontal position `x`.
 */
export function cubicBezierEase(x1: number, y1: number, x2: number, y2: number, x: number): number {
  if (x <= 0) return 0;
  if (x >= 1) return 1;
  if (x1 === y1 && x2 === y2) return x;
  const curve = (a: number, b: number, t: number) => ((1 - 3 * b + 3 * a) * t + (3 * b - 6 * a)) * t * t + 3 * a * t;
  // The curve is monotonic in x, so a bisection always finds the parameter.
  let low = 0;
  let high = 1;
  let t = x;
  for (let step = 0; step < 24; step++) {
    const error = curve(x1, x2, t) - x;
    if (Math.abs(error) < 1e-5) break;
    if (error > 0) high = t;
    else low = t;
    t = (low + high) / 2;
  }
  return curve(y1, y2, t);
}

function component(value: number | number[] | undefined, dimension: number, fallback: number): number {
  if (value === undefined) return fallback;
  if (typeof value === "number") return value;
  return value[dimension] ?? value[0] ?? fallback;
}

// ───────────────────────────── Properties ─────────────────────────────

function isKeyframes(value: unknown): value is Keyframe[] {
  return Array.isArray(value) && typeof value[0] === "object" && value[0] !== null && "t" in value[0];
}

function isShape(value: unknown): value is ShapeData {
  return typeof value === "object" && value !== null && "v" in value;
}

/** A shape keyframe wraps its contour in a one-element array. */
function unwrapShape(value: unknown): ShapeData | undefined {
  if (isShape(value)) return value;
  if (Array.isArray(value) && isShape(value[0])) return value[0];
  return undefined;
}

function asVector(value: unknown): number[] | undefined {
  if (typeof value === "number") return [value];
  if (Array.isArray(value) && (value.length === 0 || typeof value[0] === "number")) return value as number[];
  return undefined;
}

/** The keyframe segment `frame` falls in, with how far through it (0..1, before easing). */
function segmentAt(keyframes: Keyframe[], frame: number): { from: Keyframe; to: Keyframe | undefined; progress: number } {
  const first = keyframes[0]!;
  if (frame <= first.t || keyframes.length === 1) return { from: first, to: undefined, progress: 0 };
  let low = 0;
  let high = keyframes.length - 1;
  while (high - low > 1) {
    const middle = (low + high) >> 1;
    if (keyframes[middle]!.t <= frame) low = middle;
    else high = middle;
  }
  const from = keyframes[low]!;
  const to = keyframes[high]!;
  if (frame >= to.t) return { from: to, to: undefined, progress: 0 };
  return { from, to, progress: (frame - from.t) / (to.t - from.t) };
}

/** The value a segment ends on: its own `e`, or the next keyframe's start. */
function endValue(from: Keyframe, to: Keyframe): unknown {
  return from.e ?? to.s;
}

/** The value a keyframe holds when the animation rests on it (the last keyframe often has no `s`). */
function restValue(keyframes: Keyframe[], keyframe: Keyframe): unknown {
  if (keyframe.s !== undefined) return keyframe.s;
  const index = keyframes.indexOf(keyframe);
  return index > 0 ? (keyframes[index - 1]!.e ?? keyframes[index - 1]!.s) : undefined;
}

/** A numeric property (1 or more dimensions) at `frame`. */
export function vectorAt(property: Property | undefined, frame: number, fallback: number[]): number[] {
  if (!property) return fallback;
  const value = property.k;
  if (!isKeyframes(value)) return asVector(value) ?? fallback;

  const { from, to, progress } = segmentAt(value, frame);
  const start = asVector(to ? from.s : restValue(value, from));
  if (!start) return fallback;
  if (!to || from.h === 1) return start;
  const end = asVector(endValue(from, to)) ?? start;

  const eased = (dimension: number) =>
    cubicBezierEase(component(from.o?.x, dimension, 0), component(from.o?.y, dimension, 0), component(from.i?.x, dimension, 1), component(from.i?.y, dimension, 1), progress);

  // A motion path: the point travels along a bezier between the two positions.
  if (from.to && from.ti && (from.to.some((n) => n !== 0) || from.ti.some((n) => n !== 0))) {
    const t = eased(0);
    const u = 1 - t;
    return start.map((p0, dimension) => {
      const p3 = end[dimension] ?? p0;
      const p1 = p0 + (from.to![dimension] ?? 0);
      const p2 = p3 + (from.ti![dimension] ?? 0);
      return u * u * u * p0 + 3 * u * u * t * p1 + 3 * u * t * t * p2 + t * t * t * p3;
    });
  }
  return start.map((a, dimension) => a + ((end[dimension] ?? a) - a) * eased(dimension));
}

export function numberAt(property: Property | undefined, frame: number, fallback: number): number {
  return vectorAt(property, frame, [fallback])[0] ?? fallback;
}

/** A path property at `frame`; morphs between keyframes with the same number of vertices. */
export function shapeAt(property: Property | undefined, frame: number): ShapeData | undefined {
  if (!property) return undefined;
  const value = property.k;
  if (!isKeyframes(value)) return unwrapShape(value);

  const { from, to, progress } = segmentAt(value, frame);
  const start = unwrapShape(to ? from.s : restValue(value, from));
  if (!start || !to || from.h === 1) return start;
  const end = unwrapShape(endValue(from, to));
  if (!end || end.v.length !== start.v.length) return start;

  const t = cubicBezierEase(component(from.o?.x, 0, 0), component(from.o?.y, 0, 0), component(from.i?.x, 0, 1), component(from.i?.y, 0, 1), progress);
  const mix = (a: number[][], b: number[][]) => a.map((point, index) => [point[0]! + (b[index]![0]! - point[0]!) * t, point[1]! + (b[index]![1]! - point[1]!) * t]);
  return { v: mix(start.v, end.v), i: mix(start.i, end.i), o: mix(start.o, end.o), c: start.c };
}

// ───────────────────────────── Matrices ─────────────────────────────

/** A 2D affine transform `[a, b, c, d, e, f]`, as canvas `setTransform` takes it. */
export type Matrix = readonly [number, number, number, number, number, number];

export const IDENTITY: Matrix = [1, 0, 0, 1, 0, 0];

/** `m` then-applied-to the result of `n`: a point goes through `n` first. */
export function multiply(m: Matrix, n: Matrix): Matrix {
  return [
    m[0] * n[0] + m[2] * n[1],
    m[1] * n[0] + m[3] * n[1],
    m[0] * n[2] + m[2] * n[3],
    m[1] * n[2] + m[3] * n[3],
    m[0] * n[4] + m[2] * n[5] + m[4],
    m[1] * n[4] + m[3] * n[5] + m[5],
  ];
}

export function applyToPoint(m: Matrix, x: number, y: number): [number, number] {
  return [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
}

/** How much a matrix enlarges lengths (the geometric mean of its two axes). */
export function scaleOf(m: Matrix): number {
  return Math.sqrt(Math.abs(m[0] * m[3] - m[1] * m[2]));
}

const RADIANS = Math.PI / 180;

/** The matrix of a Lottie transform (layer or shape group) at `frame`. */
export function transformMatrix(transform: Transform | undefined, frame: number): Matrix {
  if (!transform) return IDENTITY;
  const anchor = vectorAt(transform.a, frame, [0, 0]);
  const position = transform.p?.s
    ? [numberAt(transform.p.x, frame, 0), numberAt(transform.p.y, frame, 0)]
    : vectorAt(transform.p, frame, [0, 0]);
  const scale = vectorAt(transform.s, frame, [100, 100]);
  const rotation = numberAt(transform.r, frame, 0) * RADIANS;
  const skew = numberAt(transform.sk, frame, 0) * RADIANS;

  let matrix: Matrix = [1, 0, 0, 1, position[0] ?? 0, position[1] ?? 0];
  if (rotation !== 0) {
    const cos = Math.cos(rotation);
    const sin = Math.sin(rotation);
    matrix = multiply(matrix, [cos, sin, -sin, cos, 0, 0]);
  }
  if (skew !== 0) {
    const axis = numberAt(transform.sa, frame, 0) * RADIANS;
    const cos = Math.cos(axis);
    const sin = Math.sin(axis);
    matrix = multiply(matrix, multiply(multiply([cos, sin, -sin, cos, 0, 0], [1, 0, Math.tan(-skew), 1, 0, 0]), [cos, -sin, sin, cos, 0, 0]));
  }
  const sx = (scale[0] ?? 100) / 100;
  const sy = (scale[1] ?? scale[0] ?? 100) / 100;
  matrix = multiply(matrix, [sx, 0, 0, sy, 0, 0]);
  return multiply(matrix, [1, 0, 0, 1, -(anchor[0] ?? 0), -(anchor[1] ?? 0)]);
}

// ───────────────────────────── Parametric shapes ─────────────────────────────

/** How far a bezier handle sits from its vertex to draw a quarter circle. */
const KAPPA = 0.5519;

export function ellipseShape(center: number[], size: number[]): ShapeData {
  const cx = center[0] ?? 0;
  const cy = center[1] ?? 0;
  const rx = (size[0] ?? 0) / 2;
  const ry = (size[1] ?? 0) / 2;
  const hx = rx * KAPPA;
  const hy = ry * KAPPA;
  return {
    c: true,
    v: [
      [cx, cy - ry],
      [cx + rx, cy],
      [cx, cy + ry],
      [cx - rx, cy],
    ],
    i: [
      [-hx, 0],
      [0, -hy],
      [hx, 0],
      [0, hy],
    ],
    o: [
      [hx, 0],
      [0, hy],
      [-hx, 0],
      [0, -hy],
    ],
  };
}

export function rectShape(center: number[], size: number[], roundness: number): ShapeData {
  const cx = center[0] ?? 0;
  const cy = center[1] ?? 0;
  const hw = (size[0] ?? 0) / 2;
  const hh = (size[1] ?? 0) / 2;
  const r = Math.min(Math.max(0, roundness), hw, hh);
  if (r === 0) {
    const zero = [0, 0];
    return {
      c: true,
      v: [
        [cx + hw, cy - hh],
        [cx + hw, cy + hh],
        [cx - hw, cy + hh],
        [cx - hw, cy - hh],
      ],
      i: [zero, zero, zero, zero],
      o: [zero, zero, zero, zero],
    };
  }
  const k = r * KAPPA;
  // Clockwise from the top-right corner; every corner is a quarter circle between two vertices.
  return {
    c: true,
    v: [
      [cx + hw, cy - hh + r],
      [cx + hw, cy + hh - r],
      [cx + hw - r, cy + hh],
      [cx - hw + r, cy + hh],
      [cx - hw, cy + hh - r],
      [cx - hw, cy - hh + r],
      [cx - hw + r, cy - hh],
      [cx + hw - r, cy - hh],
    ],
    i: [
      [0, -k],
      [0, 0],
      [k, 0],
      [0, 0],
      [0, k],
      [0, 0],
      [-k, 0],
      [0, 0],
    ],
    o: [
      [0, 0],
      [0, k],
      [0, 0],
      [-k, 0],
      [0, 0],
      [0, -k],
      [0, 0],
      [k, 0],
    ],
  };
}

/** A star (alternating outer and inner radius) or a regular polygon, with straight edges. */
export function starShape(center: number[], points: number, outer: number, inner: number | undefined, rotationDegrees: number): ShapeData {
  const count = Math.max(3, Math.round(points));
  const steps = inner === undefined ? count : count * 2;
  const v: number[][] = [];
  for (let step = 0; step < steps; step++) {
    const radius = inner !== undefined && step % 2 === 1 ? inner : outer;
    const angle = (rotationDegrees - 90) * RADIANS + (step * 2 * Math.PI) / steps;
    v.push([(center[0] ?? 0) + radius * Math.cos(angle), (center[1] ?? 0) + radius * Math.sin(angle)]);
  }
  const zero = v.map(() => [0, 0]);
  return { c: true, v, i: zero, o: zero };
}

// ───────────────────────────── Trim paths ─────────────────────────────

type Cubic = [number, number, number, number, number, number, number, number];

/** The bezier segments of a contour, as absolute control points. */
function segmentsOf(shape: ShapeData): Cubic[] {
  const count = shape.v.length;
  const segments: Cubic[] = [];
  const last = shape.c ? count : count - 1;
  for (let index = 0; index < last; index++) {
    const next = (index + 1) % count;
    const a = shape.v[index]!;
    const b = shape.v[next]!;
    const out = shape.o[index] ?? [0, 0];
    const into = shape.i[next] ?? [0, 0];
    segments.push([a[0]!, a[1]!, a[0]! + out[0]!, a[1]! + out[1]!, b[0]! + into[0]!, b[1]! + into[1]!, b[0]!, b[1]!]);
  }
  return segments;
}

function pointOn(c: Cubic, t: number): [number, number] {
  const u = 1 - t;
  const a = u * u * u;
  const b = 3 * u * u * t;
  const d = 3 * u * t * t;
  const e = t * t * t;
  return [a * c[0] + b * c[2] + d * c[4] + e * c[6], a * c[1] + b * c[3] + d * c[5] + e * c[7]];
}

const LENGTH_STEPS = 16;

/** Cumulative length of a segment at evenly spaced parameters (`LENGTH_STEPS + 1` entries). */
function lengthTable(c: Cubic): number[] {
  const table = [0];
  let previous = pointOn(c, 0);
  for (let step = 1; step <= LENGTH_STEPS; step++) {
    const point = pointOn(c, step / LENGTH_STEPS);
    table.push(table[step - 1]! + Math.hypot(point[0] - previous[0], point[1] - previous[1]));
    previous = point;
  }
  return table;
}

/** The parameter at which a segment has covered `length`. */
function parameterAt(table: number[], length: number): number {
  const total = table[LENGTH_STEPS]!;
  if (length <= 0 || total === 0) return 0;
  if (length >= total) return 1;
  let step = 1;
  while (table[step]! < length) step++;
  const before = table[step - 1]!;
  return (step - 1 + (length - before) / (table[step]! - before || 1)) / LENGTH_STEPS;
}

/** The part of a cubic between parameters `t0` and `t1`. */
function subCubic(c: Cubic, t0: number, t1: number): Cubic {
  const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
  // Split at t1 and keep the first half, then split that at t0/t1 and keep the second.
  const split = (source: Cubic, t: number): [Cubic, Cubic] => {
    const x01 = lerp(source[0], source[2], t);
    const y01 = lerp(source[1], source[3], t);
    const x12 = lerp(source[2], source[4], t);
    const y12 = lerp(source[3], source[5], t);
    const x23 = lerp(source[4], source[6], t);
    const y23 = lerp(source[5], source[7], t);
    const x012 = lerp(x01, x12, t);
    const y012 = lerp(y01, y12, t);
    const x123 = lerp(x12, x23, t);
    const y123 = lerp(y12, y23, t);
    const x = lerp(x012, x123, t);
    const y = lerp(y012, y123, t);
    return [
      [source[0], source[1], x01, y01, x012, y012, x, y],
      [x, y, x123, y123, x23, y23, source[6], source[7]],
    ];
  };
  const head = t1 >= 1 ? c : split(c, t1)[0];
  return t0 <= 0 ? head : split(head, t1 > 0 ? t0 / t1 : 0)[1];
}

/** Builds open contours from a run of connected segments. */
function contourOf(segments: Cubic[]): ShapeData {
  const shape: ShapeData = { v: [], i: [], o: [], c: false };
  segments.forEach((segment, index) => {
    if (index === 0) {
      shape.v.push([segment[0], segment[1]]);
      shape.i.push([0, 0]);
    }
    shape.o.push([segment[2] - segment[0], segment[3] - segment[1]]);
    shape.v.push([segment[6], segment[7]]);
    shape.i.push([segment[4] - segment[6], segment[5] - segment[7]]);
  });
  shape.o.push([0, 0]);
  return shape;
}

/**
 * Trim paths: keeps the stretch of each contour between `start` and `end`
 * (0..1 of its length, shifted by `offset` turns; the stretch may wrap
 * around). With `sequential`, the contours count as one long path.
 */
export function trimShapes(shapes: ShapeData[], start: number, end: number, offset: number, sequential = false): ShapeData[] {
  let from = Math.min(start, end);
  let to = Math.max(start, end);
  if (to - from >= 1) return shapes;
  if (to - from <= 0) return [];
  const shift = ((offset % 1) + 1) % 1;
  from += shift;
  to += shift;
  // Up to two stretches of the unit interval once the range wraps around.
  const ranges: [number, number][] = to <= 1 ? [[from, to]] : from >= 1 ? [[from - 1, to - 1]] : [[from, 1], [0, to - 1]];

  const measured = shapes.map((shape) => {
    const segments = segmentsOf(shape);
    const tables = segments.map(lengthTable);
    return { segments, tables, length: tables.reduce((sum, table) => sum + table[LENGTH_STEPS]!, 0) };
  });
  const total = measured.reduce((sum, item) => sum + item.length, 0);

  const result: ShapeData[] = [];
  let before = 0;
  for (const item of measured) {
    for (const [a, b] of ranges) {
      // The stretch to keep, in this contour's own length.
      const keepFrom = sequential ? a * total - before : a * item.length;
      const keepTo = sequential ? b * total - before : b * item.length;
      if (keepTo <= 0 || keepFrom >= item.length) continue;
      const run: Cubic[] = [];
      let position = 0;
      item.segments.forEach((segment, index) => {
        const table = item.tables[index]!;
        const length = table[LENGTH_STEPS]!;
        const localFrom = Math.max(0, keepFrom - position);
        const localTo = Math.min(length, keepTo - position);
        if (localTo > localFrom) run.push(subCubic(segment, parameterAt(table, localFrom), parameterAt(table, localTo)));
        position += length;
      });
      if (run.length > 0) result.push(contourOf(run));
    }
    before += item.length;
  }
  return result;
}

// ───────────────────────────── Gradients ─────────────────────────────

export interface GradientStop {
  offset: number;
  /** 0-255 channels and 0-1 alpha. */
  color: [number, number, number, number];
}

/**
 * Lottie stores a gradient as a flat list: `count` colour stops
 * (`offset, r, g, b`), then optional alpha stops (`offset, alpha`).
 */
export function gradientStops(values: number[], count: number): GradientStop[] {
  const colors: { offset: number; rgb: [number, number, number] }[] = [];
  for (let index = 0; index < count; index++) {
    const base = index * 4;
    colors.push({ offset: values[base] ?? 0, rgb: [(values[base + 1] ?? 0) * 255, (values[base + 2] ?? 0) * 255, (values[base + 3] ?? 0) * 255] });
  }
  const alphas: { offset: number; alpha: number }[] = [];
  for (let index = count * 4; index + 1 < values.length; index += 2) alphas.push({ offset: values[index]!, alpha: values[index + 1]! });
  if (colors.length === 0) return [];

  const sample = <T extends { offset: number }>(stops: T[], offset: number, mix: (a: T, b: T, t: number) => T): T => {
    if (offset <= stops[0]!.offset) return stops[0]!;
    for (let index = 1; index < stops.length; index++) {
      const a = stops[index - 1]!;
      const b = stops[index]!;
      if (offset <= b.offset) return mix(a, b, b.offset === a.offset ? 0 : (offset - a.offset) / (b.offset - a.offset));
    }
    return stops[stops.length - 1]!;
  };
  const colorAt = (offset: number) =>
    sample(colors, offset, (a, b, t) => ({ offset, rgb: [a.rgb[0] + (b.rgb[0] - a.rgb[0]) * t, a.rgb[1] + (b.rgb[1] - a.rgb[1]) * t, a.rgb[2] + (b.rgb[2] - a.rgb[2]) * t] })).rgb;
  const alphaAt = (offset: number) => (alphas.length === 0 ? 1 : sample(alphas, offset, (a, b, t) => ({ offset, alpha: a.alpha + (b.alpha - a.alpha) * t })).alpha);

  const offsets = [...new Set([...colors.map((stop) => stop.offset), ...alphas.map((stop) => stop.offset)])].sort((a, b) => a - b);
  return offsets.map((offset) => {
    const rgb = colorAt(offset);
    return { offset: Math.min(1, Math.max(0, offset)), color: [rgb[0], rgb[1], rgb[2], alphaAt(offset)] };
  });
}

// ───────────────────────────── Timing ─────────────────────────────

/** The frame to show `elapsedMs` after the animation started, looping forever. */
export function frameAt(animation: Pick<Animation, "fr" | "ip" | "op">, elapsedMs: number): number {
  const length = animation.op - animation.ip;
  if (length <= 0 || animation.fr <= 0) return animation.ip;
  return animation.ip + (((elapsedMs / 1000) * animation.fr) % length);
}
