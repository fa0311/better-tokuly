/**
 * Pixel work of the cut-out editor, as pure functions on plain buffers so
 * they can be tested without a browser.
 *
 * The editor never changes the colours of the picture: what is cut away
 * lives in a separate *mask* (one byte per pixel, 255 = keep, 0 = erased).
 * Erasing, restoring, the magic removal and undo/redo all work on the mask;
 * the picture is combined with it only for display and export.
 */

/** RGBA pixels, row by row — the shape of a canvas `ImageData`. */
export interface Bitmap {
  width: number;
  height: number;
  data: Uint8ClampedArray;
}

export interface Point {
  x: number;
  y: number;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Edge length of an exported stamp, in pixels (what the store's own art uses). */
export const STAMP_SIZE = 512;

/** Pixels with less alpha than this count as empty. */
const VISIBLE_ALPHA = 8;

/** A mask that keeps everything. */
export function fullMask(width: number, height: number): Uint8Array {
  return new Uint8Array(width * height).fill(255);
}

/** The largest size with the same proportions whose longer edge is at most `limit`. */
export function fitWithin(
  width: number,
  height: number,
  limit: number,
): { width: number; height: number } {
  const scale = Math.min(1, limit / Math.max(width, height));
  return {
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(height * scale)),
  };
}

/** The four corners of a picture: where a uniform background is sure to show. */
export function cornerSeeds(width: number, height: number): Point[] {
  return [
    { x: 0, y: 0 },
    { x: width - 1, y: 0 },
    { x: 0, y: height - 1 },
    { x: width - 1, y: height - 1 },
  ];
}

/**
 * Magic removal: erases, in `mask`, every area of similar colour that is
 * connected to one of the `seeds` (a flood fill). `tolerance` (0-255) is how
 * far a colour channel may differ from the seed's. Pixels just outside the
 * tolerance fade out, which softens the cut edge.
 *
 * Returns the number of pixels that were erased.
 */
export function floodErase(
  source: Bitmap,
  mask: Uint8Array,
  seeds: Point[],
  tolerance: number,
): number {
  const { width, height, data } = source;
  const visited = new Uint8Array(width * height);
  const stack = new Int32Array(width * height);
  // Just past the tolerance, pixels are only partly background (anti-aliased edges).
  const fade = Math.max(8, tolerance * 0.5);
  let erased = 0;
  let fill = 0;

  for (const seed of seeds) {
    const x0 = Math.round(seed.x);
    const y0 = Math.round(seed.y);
    if (x0 < 0 || y0 < 0 || x0 >= width || y0 >= height) continue;
    const start = y0 * width + x0;
    // Already cleared by an earlier seed: the same background.
    if (fill > 0 && mask[start] === 0) continue;
    // Each seed fills on its own: what one seed's colour rejects, another's may accept.
    fill++;
    if (fill > 255) break;
    const r = data[start * 4]!;
    const g = data[start * 4 + 1]!;
    const b = data[start * 4 + 2]!;
    const seedEmpty = data[start * 4 + 3]! < VISIBLE_ALPHA;

    /** 0 = the seed's colour … ≥ 1 = a different colour. */
    const difference = (index: number): number => {
      const offset = index * 4;
      const empty = data[offset + 3]! < VISIBLE_ALPHA;
      // An already transparent picture: its empty area is the background.
      if (seedEmpty || empty) return seedEmpty && empty ? 0 : 2;
      const distance = Math.max(
        Math.abs(data[offset]! - r),
        Math.abs(data[offset + 1]! - g),
        Math.abs(data[offset + 2]! - b),
      );
      return distance <= tolerance ? 0 : (distance - tolerance) / fade;
    };

    let size = 0;
    stack[size++] = start;
    visited[start] = fill;
    const visit = (index: number) => {
      if (visited[index] === fill) return;
      visited[index] = fill;
      stack[size++] = index;
    };
    while (size > 0) {
      const index = stack[--size]!;
      const amount = difference(index);
      if (amount >= 1) continue;
      if (amount > 0) {
        // Edge pixel: partly erased, and the fill does not spread through it.
        mask[index] = Math.min(mask[index]!, Math.round(amount * 255));
        continue;
      }
      if (mask[index]! > 0) erased++;
      mask[index] = 0;
      const x = index % width;
      if (x > 0) visit(index - 1);
      if (x < width - 1) visit(index + 1);
      if (index >= width) visit(index - width);
      if (index < width * (height - 1)) visit(index + width);
    }
  }
  return erased;
}

/**
 * One brush stroke from `from` to `to` (a dot when they are equal) on the
 * mask: `erase` cuts away, `restore` brings the picture back. The outer part
 * of the brush is soft. Returns the touched area, for redrawing only that.
 */
export function paintMask(
  mask: Uint8Array,
  width: number,
  height: number,
  from: Point,
  to: Point,
  radius: number,
  mode: "erase" | "restore",
): Rect {
  const reach = radius + 1;
  const left = Math.max(0, Math.floor(Math.min(from.x, to.x) - reach));
  const top = Math.max(0, Math.floor(Math.min(from.y, to.y) - reach));
  const right = Math.min(width - 1, Math.ceil(Math.max(from.x, to.x) + reach));
  const bottom = Math.min(height - 1, Math.ceil(Math.max(from.y, to.y) + reach));
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const lengthSquared = dx * dx + dy * dy;
  // The soft rim is a fifth of the brush, at least one pixel.
  const core = Math.max(0, radius - Math.max(1, radius * 0.2));

  for (let y = top; y <= bottom; y++) {
    for (let x = left; x <= right; x++) {
      // Distance from the pixel centre to the stroke's centre line.
      const t =
        lengthSquared === 0
          ? 0
          : Math.min(1, Math.max(0, ((x - from.x) * dx + (y - from.y) * dy) / lengthSquared));
      const distance = Math.hypot(x - (from.x + dx * t), y - (from.y + dy * t));
      if (distance >= radius) continue;
      const strength = distance <= core ? 1 : (radius - distance) / (radius - core);
      const index = y * width + x;
      const current = mask[index]!;
      mask[index] =
        mode === "erase"
          ? Math.min(current, Math.round(255 * (1 - strength)))
          : Math.max(current, Math.round(255 * strength));
    }
  }
  return {
    x: left,
    y: top,
    width: Math.max(0, right - left + 1),
    height: Math.max(0, bottom - top + 1),
  };
}

/**
 * Writes `source` into `out` (same size) with its alpha scaled by the mask —
 * the cut-out picture. With `area`, only that part is written.
 */
export function applyMask(
  source: Bitmap,
  mask: Uint8Array,
  out: Uint8ClampedArray,
  area?: Rect,
): void {
  const { data, width, height } = source;
  const left = Math.max(0, area?.x ?? 0);
  const top = Math.max(0, area?.y ?? 0);
  const right = Math.min(width, area ? area.x + area.width : width);
  const bottom = Math.min(height, area ? area.y + area.height : height);
  for (let y = top; y < bottom; y++) {
    for (let index = y * width + left, end = y * width + right; index < end; index++) {
      const offset = index * 4;
      out[offset] = data[offset]!;
      out[offset + 1] = data[offset + 1]!;
      out[offset + 2] = data[offset + 2]!;
      out[offset + 3] = (data[offset + 3]! * mask[index]! + 127) / 255;
    }
  }
}

/** The smallest rectangle holding every visible pixel, or `null` when nothing is left. */
export function contentBounds(bitmap: Bitmap, mask?: Uint8Array): Rect | null {
  const { width, height, data } = bitmap;
  let left = width;
  let top = height;
  let right = -1;
  let bottom = -1;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const index = y * width + x;
      const alpha = mask ? (data[index * 4 + 3]! * mask[index]!) / 255 : data[index * 4 + 3]!;
      if (alpha < VISIBLE_ALPHA) continue;
      if (x < left) left = x;
      if (x > right) right = x;
      if (y < top) top = y;
      if (y > bottom) bottom = y;
    }
  }
  return right < 0 ? null : { x: left, y: top, width: right - left + 1, height: bottom - top + 1 };
}

/** Where a picture sits in the square frame: `scale` frame pixels per picture pixel, then the offset of its top-left corner. */
export interface Placement {
  scale: number;
  x: number;
  y: number;
}

/** Centres `content` (in picture pixels) in a square frame, as large as fits inside `margin`. */
export function fitPlacement(content: Rect, frame: number, margin: number): Placement {
  const room = Math.max(1, frame - margin * 2);
  const scale = room / Math.max(content.width, content.height, 1);
  return {
    scale,
    x: (frame - content.width * scale) / 2 - content.x * scale,
    y: (frame - content.height * scale) / 2 - content.y * scale,
  };
}

/** Zooms a placement by `factor` around a point of the frame (the spot under the fingers stays put). */
export function zoomPlacement(
  placement: Placement,
  factor: number,
  around: Point,
  min: number,
  max: number,
): Placement {
  const scale = Math.min(max, Math.max(min, placement.scale * factor));
  const applied = scale / placement.scale;
  return {
    scale,
    x: around.x - (around.x - placement.x) * applied,
    y: around.y - (around.y - placement.y) * applied,
  };
}

/** A point of the frame, in picture pixels. */
export function toPicture(placement: Placement, point: Point): Point {
  return {
    x: (point.x - placement.x) / placement.scale,
    y: (point.y - placement.y) / placement.scale,
  };
}

/**
 * Squared distance of every pixel to the nearest pixel whose alpha is at
 * least `threshold` (0 inside the shape). Exact, in linear time
 * (Felzenszwalb & Huttenlocher's two-pass transform).
 */
export function distanceToShape(bitmap: Bitmap, threshold = 128): Float64Array {
  const { width, height, data } = bitmap;
  const FAR = 1e12;
  const grid = new Float64Array(width * height);
  for (let index = 0; index < grid.length; index++)
    grid[index] = data[index * 4 + 3]! >= threshold ? 0 : FAR;

  const length = Math.max(width, height);
  const f = new Float64Array(length);
  const d = new Float64Array(length);
  const v = new Int32Array(length);
  const z = new Float64Array(length + 1);

  const transform = (n: number) => {
    let k = 0;
    v[0] = 0;
    z[0] = -Infinity;
    z[1] = Infinity;
    for (let q = 1; q < n; q++) {
      let s = 0;
      for (;;) {
        const p = v[k]!;
        s = (f[q]! + q * q - (f[p]! + p * p)) / (2 * q - 2 * p);
        if (s > z[k]!) break;
        k--;
      }
      k++;
      v[k] = q;
      z[k] = s;
      z[k + 1] = Infinity;
    }
    k = 0;
    for (let q = 0; q < n; q++) {
      while (z[k + 1]! < q) k++;
      const p = v[k]!;
      d[q] = (q - p) * (q - p) + f[p]!;
    }
  };

  for (let x = 0; x < width; x++) {
    for (let y = 0; y < height; y++) f[y] = grid[y * width + x]!;
    transform(height);
    for (let y = 0; y < height; y++) grid[y * width + x] = d[y]!;
  }
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) f[x] = grid[y * width + x]!;
    transform(width);
    for (let x = 0; x < width; x++) grid[y * width + x] = d[x]!;
  }
  return grid;
}

/**
 * Marks what lies outside the shape: the empty pixels (alpha below
 * `threshold`) that are connected to the border of the picture. Holes
 * enclosed by the shape are not outside.
 */
export function outsideOfShape(bitmap: Bitmap, threshold = 128): Uint8Array {
  const { width, height, data } = bitmap;
  const outside = new Uint8Array(width * height);
  const stack = new Int32Array(width * height);
  let size = 0;
  const visit = (index: number) => {
    if (outside[index] || data[index * 4 + 3]! >= threshold) return;
    outside[index] = 1;
    stack[size++] = index;
  };
  for (let x = 0; x < width; x++) {
    visit(x);
    visit((height - 1) * width + x);
  }
  for (let y = 0; y < height; y++) {
    visit(y * width);
    visit(y * width + width - 1);
  }
  while (size > 0) {
    const index = stack[--size]!;
    const x = index % width;
    if (x > 0) visit(index - 1);
    if (x < width - 1) visit(index + 1);
    if (index >= width) visit(index - width);
    if (index < width * (height - 1)) visit(index + width);
  }
  return outside;
}

/**
 * The sticker look: paints an outline of `thickness` pixels in `color`
 * around the outer edge of the shape, underneath the picture, with a smooth
 * rim. Holes inside the shape stay open: what was erased there is not
 * painted over. Changes `bitmap` in place.
 */
export function addOutline(
  bitmap: Bitmap,
  thickness: number,
  color: readonly [number, number, number] = [255, 255, 255],
): void {
  if (thickness <= 0) return;
  const { width, height, data } = bitmap;
  const distances = distanceToShape(bitmap);
  const outside = outsideOfShape(bitmap);
  const touchesOutside = (index: number) => {
    const x = index % width;
    return (
      (x > 0 && outside[index - 1] === 1) ||
      (x < width - 1 && outside[index + 1] === 1) ||
      (index >= width && outside[index - width] === 1) ||
      (index < width * (height - 1) && outside[index + width] === 1)
    );
  };
  for (let index = 0, offset = 0; index < distances.length; index++, offset += 4) {
    const alpha = data[offset + 3]! / 255;
    if (alpha >= 1) continue;
    // Outside, the outline fades out at its thickness. The shape's own soft outer edge sits fully on it.
    const coverage = outside[index]
      ? Math.min(1, Math.max(0, thickness + 0.5 - Math.sqrt(distances[index]!)))
      : touchesOutside(index)
        ? 1
        : 0;
    if (coverage <= 0) continue;
    // The picture over the outline colour.
    const outAlpha = alpha + coverage * (1 - alpha);
    data[offset] = (data[offset]! * alpha + color[0] * coverage * (1 - alpha)) / outAlpha;
    data[offset + 1] = (data[offset + 1]! * alpha + color[1] * coverage * (1 - alpha)) / outAlpha;
    data[offset + 2] = (data[offset + 2]! * alpha + color[2] * coverage * (1 - alpha)) / outAlpha;
    data[offset + 3] = outAlpha * 255;
  }
}

/** Undo/redo over snapshots (the editor stores copies of the mask). */
export class History<T> {
  private past: T[] = [];
  private future: T[] = [];

  constructor(
    private current: T,
    private readonly limit = 30,
  ) {}

  get canUndo(): boolean {
    return this.past.length > 0;
  }

  get canRedo(): boolean {
    return this.future.length > 0;
  }

  /** Records a new state; whatever was undone before can no longer be redone. */
  push(state: T): void {
    this.past.push(this.current);
    if (this.past.length > this.limit) this.past.shift();
    this.current = state;
    this.future = [];
  }

  /** Replaces the latest state without adding a step (a setting of the last action was adjusted). */
  amend(state: T): void {
    this.current = state;
  }

  /** Steps back and returns the state to show, or `undefined` at the beginning. */
  undo(): T | undefined {
    const previous = this.past.pop();
    if (previous === undefined) return undefined;
    this.future.push(this.current);
    this.current = previous;
    return previous;
  }

  redo(): T | undefined {
    const next = this.future.pop();
    if (next === undefined) return undefined;
    this.past.push(this.current);
    this.current = next;
    return next;
  }
}
