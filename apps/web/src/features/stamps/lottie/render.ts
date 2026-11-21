/**
 * Draws one frame of a Lottie animation on a 2D canvas.
 *
 * Supported: shape layers (paths, rectangles, ellipses, stars, fills,
 * strokes with dashes, linear and radial gradients, trim paths, nested
 * groups), solids, images, precompositions with time remapping, parenting,
 * masks and alpha mattes. Not supported (drawn without them): text layers,
 * expressions, layer effects, repeaters, merge paths, rounded-corner
 * modifiers; luma mattes are treated as alpha mattes.
 */
import {
  type Animation,
  type Asset,
  applyToPoint,
  ellipseShape,
  gradientStops,
  IDENTITY,
  type Layer,
  type Matrix,
  multiply,
  numberAt,
  rectShape,
  type ShapeData,
  type ShapeItem,
  scaleOf,
  shapeAt,
  starShape,
  transformMatrix,
  trimShapes,
  vectorAt,
} from "./animation";

type Context = CanvasRenderingContext2D;

/** An animation ready to draw: the JSON plus its decoded images by asset ID. */
export interface LoadedAnimation {
  animation: Animation;
  images: Map<string, CanvasImageSource>;
}

interface Scene {
  loaded: LoadedAnimation;
  assets: Map<string, Asset>;
  /** Spare canvases for mattes, reused between frames. */
  scratch: HTMLCanvasElement[];
}

const scenes = new WeakMap<LoadedAnimation, Scene>();

function sceneOf(loaded: LoadedAnimation): Scene {
  let scene = scenes.get(loaded);
  if (!scene) {
    scene = { loaded, assets: new Map((loaded.animation.assets ?? []).map((asset) => [asset.id, asset])), scratch: [] };
    scenes.set(loaded, scene);
  }
  return scene;
}

/**
 * Draws `frame` of the animation into the `width` × `height` area at the
 * top-left of the canvas (in canvas pixels), scaled to fit and centred.
 */
export function drawFrame(context: Context, loaded: LoadedAnimation, frame: number, width: number, height: number): void {
  const { animation } = loaded;
  const scale = Math.min(width / animation.w, height / animation.h);
  const root: Matrix = [scale, 0, 0, scale, (width - animation.w * scale) / 2, (height - animation.h * scale) / 2];
  context.setTransform(1, 0, 0, 1, 0, 0);
  context.clearRect(0, 0, width, height);
  context.save();
  context.beginPath();
  context.rect(root[4], root[5], animation.w * scale, animation.h * scale);
  context.clip();
  drawLayers(context, sceneOf(loaded), animation.layers, frame, root, 1, 0);
  context.restore();
}

// ───────────────────────────── Layers ─────────────────────────────

/** A layer's own transform combined with those of its parents. */
function layerMatrix(layer: Layer, layers: Layer[], frame: number): Matrix {
  let matrix = transformMatrix(layer.ks, frame);
  let parent = layer.parent;
  // Parent chains are short; the counter only guards against a cycle in a broken file.
  for (let depth = 0; parent !== undefined && depth < 32; depth++) {
    const above = layers.find((candidate) => candidate.ind === parent);
    if (!above) break;
    matrix = multiply(transformMatrix(above.ks, frame), matrix);
    parent = above.parent;
  }
  return matrix;
}

function drawLayers(context: Context, scene: Scene, layers: Layer[], frame: number, base: Matrix, alpha: number, depth: number): void {
  if (depth > 8) return;
  for (let index = layers.length - 1; index >= 0; index--) {
    const layer = layers[index]!;
    // A matte layer is only drawn through the layer it cuts out.
    if (layer.hd || layer.td || frame < layer.ip || frame >= layer.op) continue;
    const matte = layer.tt ? (layer.tp !== undefined ? layers.find((candidate) => candidate.ind === layer.tp) : layers[index - 1]) : undefined;
    if (matte) drawMatted(context, scene, layers, layer, matte, frame, base, alpha, depth);
    else drawLayer(context, scene, layers, layer, frame, base, alpha, depth);
  }
}

function scratchCanvas(scene: Scene, width: number, height: number): HTMLCanvasElement {
  const canvas = scene.scratch.pop() ?? document.createElement("canvas");
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  return canvas;
}

function drawMatted(context: Context, scene: Scene, layers: Layer[], layer: Layer, matte: Layer, frame: number, base: Matrix, alpha: number, depth: number): void {
  const { width, height } = context.canvas;
  const content = scratchCanvas(scene, width, height);
  const shape = scratchCanvas(scene, width, height);
  const contentContext = content.getContext("2d");
  const shapeContext = shape.getContext("2d");
  if (contentContext && shapeContext) {
    for (const target of [contentContext, shapeContext]) {
      target.setTransform(1, 0, 0, 1, 0, 0);
      target.globalCompositeOperation = "source-over";
      target.globalAlpha = 1;
      target.clearRect(0, 0, width, height);
    }
    drawLayer(contentContext, scene, layers, layer, frame, base, alpha, depth);
    if (frame >= matte.ip && frame < matte.op) drawLayer(shapeContext, scene, layers, matte, frame, base, 1, depth);
    contentContext.setTransform(1, 0, 0, 1, 0, 0);
    contentContext.globalAlpha = 1;
    // Even matte types keep what the matte covers; odd ones are inverted.
    contentContext.globalCompositeOperation = layer.tt === 2 || layer.tt === 4 ? "destination-out" : "destination-in";
    contentContext.drawImage(shape, 0, 0);
    contentContext.globalCompositeOperation = "source-over";
    context.save();
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.globalAlpha = 1;
    context.drawImage(content, 0, 0);
    context.restore();
  }
  scene.scratch.push(content, shape);
}

const BLEND_MODES: Record<number, GlobalCompositeOperation> = {
  1: "multiply",
  2: "screen",
  3: "overlay",
  4: "darken",
  5: "lighten",
  6: "color-dodge",
  7: "color-burn",
  8: "hard-light",
  9: "soft-light",
  10: "difference",
  11: "exclusion",
};

function drawLayer(context: Context, scene: Scene, layers: Layer[], layer: Layer, frame: number, base: Matrix, alpha: number, depth: number): void {
  const opacity = alpha * (numberAt(layer.ks?.o, frame, 100) / 100);
  if (opacity <= 0) return;
  const matrix = multiply(base, layerMatrix(layer, layers, frame));

  context.save();
  context.setTransform(matrix[0], matrix[1], matrix[2], matrix[3], matrix[4], matrix[5]);
  if (layer.bm) context.globalCompositeOperation = BLEND_MODES[layer.bm] ?? "source-over";
  clipToMasks(context, layer, frame);

  if (layer.ty === 4 && layer.shapes) {
    const operations: DrawOperation[] = [];
    collectShapes(layer.shapes, IDENTITY, 1, frame, operations);
    for (let index = operations.length - 1; index >= 0; index--) paint(context, operations[index]!, frame, opacity);
  } else if (layer.ty === 1 && layer.sc) {
    context.globalAlpha = opacity;
    context.fillStyle = layer.sc;
    context.fillRect(0, 0, layer.sw ?? 0, layer.sh ?? 0);
  } else if (layer.ty === 2 && layer.refId) {
    const image = scene.loaded.images.get(layer.refId);
    const asset = scene.assets.get(layer.refId);
    if (image && asset) {
      context.globalAlpha = opacity;
      context.drawImage(image, 0, 0, asset.w ?? 0, asset.h ?? 0);
    }
  } else if (layer.ty === 0 && layer.refId) {
    const composition = scene.assets.get(layer.refId);
    if (composition?.layers) {
      context.beginPath();
      context.rect(0, 0, layer.w ?? scene.loaded.animation.w, layer.h ?? scene.loaded.animation.h);
      context.clip();
      // Inside the composition, time runs from the layer's start; a time remap replaces it (in seconds).
      const local = layer.tm ? numberAt(layer.tm, frame, 0) * scene.loaded.animation.fr : (frame - (layer.st ?? 0)) / (layer.sr || 1);
      drawLayers(context, scene, composition.layers, local, matrix, opacity, depth + 1);
    }
  }
  context.restore();
}

/** Restricts drawing to the layer's masks (the canvas transform is the layer's). */
function clipToMasks(context: Context, layer: Layer, frame: number): void {
  const masks = layer.masksProperties?.filter((mask) => mask.mode !== "n");
  if (!masks?.length) return;
  const added = masks.filter((mask) => (mask.mode === "a" || mask.mode === undefined) && !mask.inv);
  if (added.length > 0) {
    context.beginPath();
    for (const mask of added) traceContour(context, shapeAt(mask.pt, frame), IDENTITY);
    context.clip();
  }
  for (const mask of masks) {
    if (added.includes(mask)) continue;
    const shape = shapeAt(mask.pt, frame);
    if (!shape) continue;
    const inverted = mask.mode === "s" ? !mask.inv : Boolean(mask.inv);
    context.beginPath();
    // Everything but the shape: a huge rectangle with the shape as a hole.
    if (inverted) context.rect(-1e5, -1e5, 2e5, 2e5);
    traceContour(context, shape, IDENTITY);
    context.clip(inverted ? "evenodd" : "nonzero");
  }
}

// ───────────────────────────── Shapes ─────────────────────────────

interface PathEntry {
  /** One or more contours (more after trimming). */
  shapes: ShapeData[];
  /** Maps the contours into the layer's space. */
  matrix: Matrix;
}

/** A fill or stroke and the paths it paints. */
interface DrawOperation {
  style: ShapeItem;
  /** Shared with the group: only the first `count` entries were above the style. */
  paths: PathEntry[];
  count: number;
  matrix: Matrix;
  opacity: number;
}

/**
 * Walks a group's contents top to bottom. Paths pile up; every fill or
 * stroke paints the paths above it in its group (including nested groups').
 * The operations come out top-most first, so they are painted in reverse.
 */
function collectShapes(items: ShapeItem[], parent: Matrix, parentOpacity: number, frame: number, operations: DrawOperation[]): PathEntry[] {
  const transform = items.find((item) => item.ty === "tr");
  const matrix = transform ? multiply(parent, transformMatrix(transform, frame)) : parent;
  const opacity = parentOpacity * (transform ? numberAt(transform.o, frame, 100) / 100 : 1);
  const paths: PathEntry[] = [];
  const trims: ShapeItem[] = [];

  for (const item of items) {
    if (item.hd) continue;
    switch (item.ty) {
      case "gr":
        if (item.it) paths.push(...collectShapes(item.it, matrix, opacity, frame, operations));
        break;
      case "sh": {
        const shape = shapeAt(item.ks, frame);
        if (shape) paths.push({ shapes: [shape], matrix });
        break;
      }
      case "rc":
        paths.push({ shapes: [rectShape(vectorAt(item.p, frame, [0, 0]), vectorAt(item.s, frame, [0, 0]), numberAt(item.r, frame, 0))], matrix });
        break;
      case "el":
        paths.push({ shapes: [ellipseShape(vectorAt(item.p, frame, [0, 0]), vectorAt(item.s, frame, [0, 0]))], matrix });
        break;
      case "sr":
        paths.push({
          shapes: [starShape(vectorAt(item.p, frame, [0, 0]), numberAt(item.pt, frame, 5), numberAt(item.or, frame, 0), item.sy === 2 ? undefined : numberAt(item.ir, frame, 0), numberAt(item.r, frame, 0))],
          matrix,
        });
        break;
      case "tm":
        trims.push(item);
        break;
      case "fl":
      case "st":
      case "gf":
      case "gs":
        operations.push({ style: item, paths, count: paths.length, matrix, opacity });
        break;
    }
  }

  // A trim shortens every path of its group, wherever it sits among the styles.
  for (const trim of trims) {
    const start = numberAt(trim.s, frame, 0) / 100;
    const end = numberAt(trim.e, frame, 100) / 100;
    const offset = numberAt(trim.o, frame, 0) / 360;
    if (trim.m === 2) {
      // One after another: the paths form a single line, drawn from the last path to the first.
      const all = paths.flatMap((entry) => entry.shapes.map((shape) => ({ entry, shape }))).reverse();
      const trimmed = trimShapes(all.map((item) => item.shape), start, end, offset, true);
      for (const entry of paths) entry.shapes = [];
      // Trimmed contours no longer map to their source one to one; they share the first matrix in practice.
      if (paths[0]) paths[0].shapes = trimmed;
    } else {
      for (const entry of paths) entry.shapes = trimShapes(entry.shapes, start, end, offset);
    }
  }
  return paths;
}

/** Adds a contour to the current canvas path, with its points mapped through `matrix`. */
function traceContour(context: Context, shape: ShapeData | undefined, matrix: Matrix): void {
  if (!shape || shape.v.length === 0) return;
  const count = shape.v.length;
  const first = shape.v[0]!;
  const [startX, startY] = applyToPoint(matrix, first[0]!, first[1]!);
  context.moveTo(startX, startY);
  const last = shape.c ? count : count - 1;
  for (let index = 0; index < last; index++) {
    const next = (index + 1) % count;
    const from = shape.v[index]!;
    const to = shape.v[next]!;
    const out = shape.o[index] ?? [0, 0];
    const into = shape.i[next] ?? [0, 0];
    const [c1x, c1y] = applyToPoint(matrix, from[0]! + out[0]!, from[1]! + out[1]!);
    const [c2x, c2y] = applyToPoint(matrix, to[0]! + into[0]!, to[1]! + into[1]!);
    const [x, y] = applyToPoint(matrix, to[0]!, to[1]!);
    context.bezierCurveTo(c1x, c1y, c2x, c2y, x, y);
  }
  if (shape.c) context.closePath();
}

function cssColor(color: number[], alpha = 1): string {
  const channel = (value: number | undefined) => Math.round(Math.min(1, Math.max(0, value ?? 0)) * 255);
  return `rgba(${channel(color[0])}, ${channel(color[1])}, ${channel(color[2])}, ${alpha})`;
}

function gradientOf(context: Context, operation: DrawOperation, frame: number): CanvasGradient | null {
  const { style, matrix } = operation;
  if (!style.g) return null;
  const start = vectorAt(style.s, frame, [0, 0]);
  const end = vectorAt(style.e, frame, [0, 0]);
  const [x0, y0] = applyToPoint(matrix, start[0] ?? 0, start[1] ?? 0);
  const [x1, y1] = applyToPoint(matrix, end[0] ?? 0, end[1] ?? 0);
  const gradient = style.t === 2 ? context.createRadialGradient(x0, y0, 0, x0, y0, Math.hypot(x1 - x0, y1 - y0)) : context.createLinearGradient(x0, y0, x1, y1);
  for (const stop of gradientStops(vectorAt(style.g.k, frame, []), style.g.p)) {
    gradient.addColorStop(stop.offset, `rgba(${Math.round(stop.color[0])}, ${Math.round(stop.color[1])}, ${Math.round(stop.color[2])}, ${stop.color[3]})`);
  }
  return gradient;
}

const LINE_CAPS: CanvasLineCap[] = ["butt", "round", "square"];
const LINE_JOINS: CanvasLineJoin[] = ["miter", "round", "bevel"];

function paint(context: Context, operation: DrawOperation, frame: number, layerOpacity: number): void {
  const { style } = operation;
  const alpha = layerOpacity * operation.opacity * (numberAt(style.o, frame, 100) / 100);
  if (alpha <= 0) return;

  context.beginPath();
  for (let index = 0; index < operation.count; index++) {
    const entry = operation.paths[index]!;
    for (const shape of entry.shapes) traceContour(context, shape, entry.matrix);
  }
  context.globalAlpha = Math.min(1, alpha);
  const paintStyle = style.ty === "gf" || style.ty === "gs" ? gradientOf(context, operation, frame) : cssColor(vectorAt(style.c, frame, [0, 0, 0]));
  if (!paintStyle) return;

  if (style.ty === "fl" || style.ty === "gf") {
    context.fillStyle = paintStyle;
    // On a fill, `r` is the fill rule as a plain number: 1 = non-zero, 2 = even-odd (holes).
    context.fill((style.r as unknown) === 2 ? "evenodd" : "nonzero");
    return;
  }
  const scale = scaleOf(operation.matrix);
  context.strokeStyle = paintStyle;
  context.lineWidth = numberAt(style.w, frame, 0) * scale;
  context.lineCap = LINE_CAPS[(style.lc ?? 2) - 1] ?? "round";
  context.lineJoin = LINE_JOINS[(style.lj ?? 2) - 1] ?? "round";
  context.miterLimit = style.ml ?? 4;
  if (Array.isArray(style.d)) {
    const dashes = style.d.filter((dash) => dash.n !== "o").map((dash) => numberAt(dash.v, frame, 0) * scale);
    const offset = style.d.find((dash) => dash.n === "o");
    // A single value means equal dashes and gaps.
    context.setLineDash(dashes.length === 1 ? [dashes[0]!, dashes[0]!] : dashes);
    context.lineDashOffset = offset ? numberAt(offset.v, frame, 0) * scale : 0;
  } else {
    context.setLineDash([]);
  }
  if (context.lineWidth > 0) context.stroke();
}
