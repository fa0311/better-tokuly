/** What the browser can tell about a video file before it is uploaded. */
export interface VideoFacts {
  durationSeconds: number;
  width: number;
  height: number;
}

function loadVideo(src: string, crossOrigin = false): Promise<HTMLVideoElement> {
  return new Promise((resolve, reject) => {
    const video = document.createElement("video");
    video.preload = "auto";
    video.muted = true;
    video.playsInline = true;
    if (crossOrigin) video.crossOrigin = "anonymous";
    video.onloadedmetadata = () => resolve(video);
    video.onerror = () => reject(new Error("VIDEO_UNREADABLE"));
    video.src = src;
  });
}

function release(video: HTMLVideoElement): void {
  video.pause();
  video.removeAttribute("src");
  video.load();
}

function seek(video: HTMLVideoElement, seconds: number): Promise<void> {
  return new Promise((resolve, reject) => {
    video.onseeked = () => resolve();
    video.onerror = () => reject(new Error("VIDEO_UNREADABLE"));
    video.currentTime = seconds;
  });
}

/** Length and picture size of a video, read from its metadata. Rejects when the browser cannot open it. */
export async function probeVideo(src: string): Promise<VideoFacts> {
  const video = await loadVideo(src);
  try {
    // Some recordings (WebM from a recorder) only know their length after a seek to the end.
    if (!Number.isFinite(video.duration)) {
      await seek(video, Number.MAX_SAFE_INTEGER);
      await seek(video, 0);
    }
    return {
      durationSeconds: Number.isFinite(video.duration) ? video.duration : 0,
      width: video.videoWidth,
      height: video.videoHeight,
    };
  } finally {
    release(video);
  }
}

/**
 * Small pictures of the video at the given moments, for a filmstrip. Gives
 * up quietly (an empty list) when the frames cannot be read: a file the
 * browser cannot decode, or a video on another origin.
 */
export async function captureFrames(
  src: string,
  times: number[],
  options: { height?: number; crossOrigin?: boolean; signal?: AbortSignal } = {},
): Promise<string[]> {
  const { height = 160, crossOrigin = false, signal } = options;
  let video: HTMLVideoElement | undefined;
  try {
    video = await loadVideo(src, crossOrigin);
    if (!video.videoWidth || !video.videoHeight) return [];
    const canvas = document.createElement("canvas");
    canvas.height = height;
    canvas.width = Math.round((video.videoWidth / video.videoHeight) * height);
    const context = canvas.getContext("2d");
    if (!context) return [];
    const frames: string[] = [];
    for (const time of times) {
      if (signal?.aborted) return [];
      await seek(video, time);
      context.drawImage(video, 0, 0, canvas.width, canvas.height);
      frames.push(canvas.toDataURL("image/jpeg", 0.6));
    }
    return frames;
  } catch {
    return [];
  } finally {
    if (video) release(video);
  }
}

type CapturingVideo = HTMLVideoElement & {
  captureStream?(): MediaStream;
  mozCaptureStream?(): MediaStream;
};

const RECORDER_TYPES = [
  "video/mp4;codecs=avc1,mp4a",
  "video/webm;codecs=vp9,opus",
  "video/webm;codecs=vp8,opus",
  "video/webm",
];

function recorderType(): string | undefined {
  if (typeof MediaRecorder === "undefined") return undefined;
  return RECORDER_TYPES.find((type) => MediaRecorder.isTypeSupported(type));
}

/** Whether this browser can cut a part out of a video (it records the part while playing it). */
export function canTrim(): boolean {
  if (typeof document === "undefined" || !recorderType()) return false;
  const video = document.createElement("video") as CapturingVideo;
  return typeof (video.captureStream ?? video.mozCaptureStream) === "function";
}

/**
 * Cuts `[start, end]` (seconds) out of a video by playing that part and
 * recording it: it takes as long as the part is. `onProgress` gets 0…1.
 */
export async function trimVideo(
  src: string,
  start: number,
  end: number,
  options: { onProgress?(fraction: number): void; signal?: AbortSignal } = {},
): Promise<File> {
  const mimeType = recorderType();
  const video = (await loadVideo(src)) as CapturingVideo;
  const capture = video.captureStream ?? video.mozCaptureStream;
  if (!mimeType || !capture) throw new Error("TRIM_UNSUPPORTED");
  // The recording must carry the sound; nobody should have to hear it twice.
  video.muted = false;
  video.volume = 0;
  try {
    await seek(video, start);
    const stream = capture.call(video);
    const recorder = new MediaRecorder(stream, { mimeType, videoBitsPerSecond: 4_000_000 });
    const chunks: Blob[] = [];
    recorder.ondataavailable = (event) => event.data.size > 0 && chunks.push(event.data);
    const stopped = new Promise<void>((resolve) => {
      recorder.onstop = () => resolve();
    });
    await new Promise<void>((resolve, reject) => {
      const finish = () => {
        video.pause();
        video.ontimeupdate = null;
        video.onended = null;
        resolve();
      };
      video.ontimeupdate = () => {
        options.onProgress?.(Math.min(1, (video.currentTime - start) / (end - start)));
        if (video.currentTime >= end) finish();
      };
      video.onended = finish;
      video.onerror = () => reject(new Error("VIDEO_UNREADABLE"));
      options.signal?.addEventListener(
        "abort",
        () => reject(new DOMException("trim aborted", "AbortError")),
        { once: true },
      );
      recorder.start(1000);
      video.play().catch(reject);
    });
    if (recorder.state !== "inactive") recorder.stop();
    await stopped;
    for (const track of stream.getTracks()) track.stop();
    const type = mimeType.split(";")[0] ?? "video/webm";
    return new File(
      [new Blob(chunks, { type })],
      `short.${type.endsWith("mp4") ? "mp4" : "webm"}`,
      {
        type,
      },
    );
  } finally {
    release(video);
  }
}
