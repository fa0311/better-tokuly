import { ImageManipulator, SaveFormat } from "expo-image-manipulator";

/** What a profile picture is cut to, as on the web: its shape and largest width. */
export interface ProfileFrame {
  /** Width / height. */
  aspect: number;
  /** Width of the uploaded image in pixels (never larger than the picked one). */
  width: number;
}

export const AVATAR_FRAME: ProfileFrame = { aspect: 1, width: 640 };
export const BANNER_FRAME: ProfileFrame = { aspect: 3, width: 1500 };

interface Size {
  width: number;
  height: number;
}

/**
 * The largest centred part of the image in the frame's shape, and the size
 * it is scaled to. The picker has usually cropped already; an uncropped
 * banner (iOS) is cut at the middle.
 */
export function framing(source: Size, frame: ProfileFrame) {
  const regionWidth = Math.min(source.width, source.height * frame.aspect);
  const regionHeight = regionWidth / frame.aspect;
  const width = Math.max(1, Math.round(Math.min(frame.width, regionWidth)));
  return {
    crop: {
      originX: Math.round((source.width - regionWidth) / 2),
      originY: Math.round((source.height - regionHeight) / 2),
      width: Math.round(regionWidth),
      height: Math.round(regionHeight),
    },
    size: { width, height: Math.max(1, Math.round(width / frame.aspect)) },
  };
}

/**
 * The picked image cut and scaled to the frame, as a JPEG file: a 200 MP
 * photo becomes a few hundred kilobytes instead of being refused by the
 * server. Big photos are scaled down while they are decoded, so the full
 * size never sits in memory.
 */
export async function fitToFrame(
  image: { uri: string } & Size,
  frame: ProfileFrame,
): Promise<{ uri: string; type: "image/jpeg" }> {
  const regionWidth = Math.min(image.width, image.height * frame.aspect);
  // The picker's size is a hint only: what was decoded is measured again below.
  const load =
    regionWidth > frame.width
      ? { maxWidth: Math.ceil((image.width * frame.width) / regionWidth) }
      : {};
  const decoded = await ImageManipulator.manipulate(image.uri, load).renderAsync();
  const { crop, size } = framing(decoded, frame);
  const framed = await ImageManipulator.manipulate(decoded).crop(crop).resize(size).renderAsync();
  const saved = await framed.saveAsync({ format: SaveFormat.JPEG, compress: 0.9 });
  return { uri: saved.uri, type: "image/jpeg" };
}
