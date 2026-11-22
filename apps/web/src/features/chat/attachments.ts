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

/** Pictures browsers can show (and the server can process) go as images; everything else as its own kind. */
export function classify(type: string): AttachmentKind {
  if (/^image\/(png|jpe?g|gif|webp|avif|heic|heif|bmp)$/i.test(type)) return "image";
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

/** Wraps a picked, pasted, dropped or recorded file for sending, measuring what the preview needs. */
export async function describeFile(
  file: Blob,
  extra: Partial<Pick<OutgoingFile, "name" | "kind" | "durationSeconds" | "waveform">> = {},
): Promise<OutgoingFile> {
  const kind = extra.kind ?? classify(file.type);
  const previewUrl = URL.createObjectURL(file);
  const name = extra.name ?? (file as File).name ?? "file";
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
