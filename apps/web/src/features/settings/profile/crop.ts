/** What a picked image is cropped to. */
export interface CropFrame {
  /** Decides the outline drawn around the frame. */
  shape: "avatar" | "banner";
  /** Width / height. */
  aspect: number;
  /** Width of the rendered image in pixels (never larger than the source region). */
  outputWidth: number;
}

/** Avatars are shown as rounded squares everywhere. */
export const AVATAR_FRAME: CropFrame = { shape: "avatar", aspect: 1, outputWidth: 640 };
/** Banners are wide strips across the top of a profile. */
export const BANNER_FRAME: CropFrame = { shape: "banner", aspect: 3, outputWidth: 1500 };

/**
 * Which part of the image is inside the frame: the image point at the middle
 * of the frame (0..1 on both axes) and the zoom, where 1 shows as much of the
 * image as the frame's shape allows.
 */
export interface CropView {
  centerX: number;
  centerY: number;
  zoom: number;
}

interface Size {
  width: number;
  height: number;
}

/** The rectangle of the image (in image pixels) that is inside the frame. */
export function visibleRegion(view: CropView, source: Size, aspect: number): { x: number; y: number; width: number; height: number } {
  const width = Math.min(source.width, source.height * aspect) / view.zoom;
  const height = width / aspect;
  return { x: view.centerX * source.width - width / 2, y: view.centerY * source.height - height / 2, width, height };
}

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

/** Keeps the zoom in range and the frame inside the image (no empty edges). */
export function clampView(view: CropView, source: Size, aspect: number, maxZoom: number): CropView {
  const zoom = clamp(view.zoom, 1, maxZoom);
  const region = visibleRegion({ ...view, zoom }, source, aspect);
  const halfWidth = region.width / 2 / source.width;
  const halfHeight = region.height / 2 / source.height;
  return { zoom, centerX: clamp(view.centerX, halfWidth, 1 - halfWidth), centerY: clamp(view.centerY, halfHeight, 1 - halfHeight) };
}

/** Renders the framed part of the image as a JPEG (transparent areas become white). */
export function renderCrop(image: HTMLImageElement, view: CropView, frame: CropFrame): Promise<Blob> {
  const region = visibleRegion(view, { width: image.naturalWidth, height: image.naturalHeight }, frame.aspect);
  const width = Math.max(1, Math.round(Math.min(frame.outputWidth, region.width)));
  const height = Math.max(1, Math.round(width / frame.aspect));
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (!context) return Promise.reject(new Error("canvas is unavailable"));
  context.fillStyle = "#ffffff";
  context.fillRect(0, 0, width, height);
  context.imageSmoothingQuality = "high";
  context.drawImage(image, region.x, region.y, region.width, region.height, 0, 0, width, height);
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error("the image could not be encoded"))), "image/jpeg", 0.9);
  });
}
