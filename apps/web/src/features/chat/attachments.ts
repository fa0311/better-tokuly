import { create, media } from "@tokuly/api";

/** What a file is sent as. */
export type AttachmentKind = "image" | "video" | "audio" | "file";

/** A file waiting to be sent with a message. */
export interface OutgoingFile {
  /** Local identity (for lists and removal). */
  id: string;
  file: Blob;
  name: string;
  kind: AttachmentKind;
  /** Object URL of the file itself, for the preview shown until the server's copy is there. */
  previewUrl: string;
  width?: number;
  height?: number;
  durationSeconds?: number;
  /** Peaks of a voice message (0-255), as the server stores them. */
  waveform?: Uint8Array;
  /** Set once the bytes are on the server. */
  assetId?: string;
}

export const PURPOSE: Record<AttachmentKind, media.AssetPurpose> = {
  image: media.AssetPurpose.CHAT_IMAGE,
  video: media.AssetPurpose.CHAT_VIDEO,
  audio: media.AssetPurpose.CHAT_AUDIO,
  file: media.AssetPurpose.CHAT_FILE,
};

const ASSET_KIND: Record<AttachmentKind, media.AssetKind> = {
  image: media.AssetKind.IMAGE,
  video: media.AssetKind.VIDEO,
  audio: media.AssetKind.AUDIO,
  file: media.AssetKind.FILE,
};

/** Picture formats the server decodes itself (and re-encodes, dropping EXIF and GPS). */
const SERVER_IMAGE = /^image\/(png|jpe?g|gif|webp)$/i;
/**
 * Photo formats the server cannot decode (iPhone HEIC, AVIF, BMP). They are
 * converted to JPEG here, before upload: a photo never goes out as a file,
 * because files keep their bytes, metadata and location included, untouched.
 */
const CONVERTED_IMAGE = /^image\/(avif|heic|heif|heic-sequence|heif-sequence|bmp)$/i;

/** Whether a picture of this type is converted to JPEG before upload. */
export function needsConversion(type: string): boolean {
  return CONVERTED_IMAGE.test(type);
}

/** A photo this browser cannot decode (e.g. HEIC outside Safari), so it cannot be sent. */
export class UnreadablePhotoError extends Error {
  constructor() {
    super("this browser cannot read the photo");
    this.name = "UnreadablePhotoError";
  }
}

/** Re-encodes a picture as JPEG through a canvas: the orientation is applied, all metadata is gone. */
export async function toJpeg(file: Blob, name: string): Promise<File> {
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file, { imageOrientation: "from-image" });
  } catch {
    throw new UnreadablePhotoError();
  }
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  canvas.getContext("2d")?.drawImage(bitmap, 0, 0);
  bitmap.close();
  const jpeg = await new Promise<Blob | null>((resolve) =>
    canvas.toBlob(resolve, "image/jpeg", 0.9),
  );
  if (!jpeg) throw new UnreadablePhotoError();
  return new File([jpeg], `${name.replace(/\.[^.]*$/, "") || "photo"}.jpg`, { type: "image/jpeg" });
}

/** Pictures the server can process (directly or after conversion) go as images; everything else as its own kind. */
export function classify(type: string): AttachmentKind {
  if (SERVER_IMAGE.test(type) || CONVERTED_IMAGE.test(type)) return "image";
  if (type.startsWith("video/")) return "video";
  if (type.startsWith("audio/")) return "audio";
  return "file";
}

function imageSize(url: string): Promise<{ width: number; height: number } | undefined> {
  return new Promise((resolve) => {
    const image = new Image();
    image.onload = () => resolve({ width: image.naturalWidth, height: image.naturalHeight });
    image.onerror = () => resolve(undefined);
    image.src = url;
  });
}

function videoSize(
  url: string,
): Promise<{ width: number; height: number; durationSeconds: number } | undefined> {
  return new Promise((resolve) => {
    const video = document.createElement("video");
    video.preload = "metadata";
    video.muted = true;
    video.onloadedmetadata = () =>
      resolve({
        width: video.videoWidth,
        height: video.videoHeight,
        durationSeconds: Number.isFinite(video.duration) ? video.duration : 0,
      });
    video.onerror = () => resolve(undefined);
    video.src = url;
  });
}

let counter = 0;

/**
 * Wraps a picked, pasted, dropped or recorded file for sending, measuring
 * what the preview needs. Photos in formats the server cannot decode are
 * converted to JPEG; one this browser cannot read rejects with
 * {@link UnreadablePhotoError}.
 */
export async function describeFile(
  picked: Blob,
  extra: Partial<Pick<OutgoingFile, "name" | "kind" | "durationSeconds" | "waveform">> = {},
): Promise<OutgoingFile> {
  const kind = extra.kind ?? classify(picked.type);
  const pickedName = extra.name ?? (picked as File).name ?? "file";
  const file =
    kind === "image" && needsConversion(picked.type) ? await toJpeg(picked, pickedName) : picked;
  const name = file === picked ? pickedName : (file as File).name;
  const previewUrl = URL.createObjectURL(file);
  const base: OutgoingFile = {
    id: `f${Date.now().toString(36)}${counter++}`,
    file,
    name,
    kind,
    previewUrl,
    durationSeconds: extra.durationSeconds,
    waveform: extra.waveform,
  };
  if (kind === "image") return { ...base, ...(await imageSize(previewUrl)) };
  if (kind === "video") {
    const size = await videoSize(previewUrl);
    return size ? { ...base, ...size } : base;
  }
  return base;
}

/** Releases the previews of files that are no longer shown. */
export function releaseFiles(files: OutgoingFile[]): void {
  for (const file of files) URL.revokeObjectURL(file.previewUrl);
}

/** The attachment a file will become, as far as the client can tell, to draw it before it is sent. */
export function localAttachment(file: OutgoingFile): media.Attachment {
  const playable = file.kind === "video" || file.kind === "audio";
  return create(media.AttachmentSchema, {
    assetId: file.assetId ?? "",
    kind: ASSET_KIND[file.kind],
    url: file.previewUrl,
    contentType: file.file.type,
    sizeBytes: BigInt(file.file.size),
    fileName: file.name,
    image:
      file.kind === "image" && file.width && file.height
        ? { url: file.previewUrl, width: file.width, height: file.height }
        : undefined,
    video: playable
      ? {
          durationSeconds: file.durationSeconds ?? 0,
          width: file.width ?? 0,
          height: file.height ?? 0,
          mp4Url: file.kind === "video" ? file.previewUrl : "",
          hasAudio: true,
        }
      : undefined,
    waveform: file.waveform ?? new Uint8Array(),
    status: media.AssetStatus.READY,
  });
}
