/**
 * The cameras of the browser for a moment: the back and the front at once
 * where the device can run both (most laptops with two cameras, some
 * phones), otherwise one after the other through the same camera.
 */

export type Facing = "environment" | "user";

export type Rig =
  | { kind: "dual"; back: MediaStream; front: MediaStream }
  | { kind: "single"; stream: MediaStream; facing: Facing }
  | { kind: "unavailable" };

export function openCamera(facing: Facing): Promise<MediaStream> {
  return navigator.mediaDevices.getUserMedia({
    video: { facingMode: { ideal: facing }, width: { ideal: 1920 }, height: { ideal: 1440 } },
    audio: false,
  });
}

export function stopStream(stream: MediaStream | undefined): void {
  for (const track of stream?.getTracks() ?? []) track.stop();
}

export function stopRig(rig: Rig | null): void {
  if (rig?.kind === "dual") {
    stopStream(rig.back);
    stopStream(rig.front);
  } else if (rig?.kind === "single") stopStream(rig.stream);
}

const deviceOf = (stream: MediaStream) => stream.getVideoTracks()[0]?.getSettings().deviceId;
const alive = (stream: MediaStream) =>
  stream.getVideoTracks().every((track) => track.readyState === "live");

/**
 * Opens the back camera and, when there is a second one, the front one next
 * to it. A phone that stops one camera to start the other (or hands out
 * the same camera twice) gets one camera, switched between the two shots.
 */
export async function openRig(): Promise<Rig> {
  if (!navigator.mediaDevices?.getUserMedia) return { kind: "unavailable" };
  let back: MediaStream;
  try {
    back = await openCamera("environment");
  } catch {
    return { kind: "unavailable" };
  }
  const devices = await navigator.mediaDevices.enumerateDevices().catch(() => []);
  if (devices.filter((device) => device.kind === "videoinput").length > 1) {
    try {
      const front = await openCamera("user");
      await new Promise((resolve) => setTimeout(resolve, 300));
      if (deviceOf(front) !== deviceOf(back) && alive(back) && alive(front))
        return { kind: "dual", back, front };
      stopStream(front);
      if (!alive(back)) {
        stopStream(back);
        back = await openCamera("environment");
      }
    } catch {
      // One camera at a time, then.
    }
  }
  return { kind: "single", stream: back, facing: "environment" };
}

/** The longest edge of a picture sent: the media server makes the renditions. */
const MAX_EDGE = 2048;

/** The frame the video shows now, as a JPEG (mirrored like the preview for a selfie). */
export function snap(video: HTMLVideoElement, mirror: boolean): Promise<Blob | null> {
  const scale = Math.min(1, MAX_EDGE / Math.max(video.videoWidth, video.videoHeight, 1));
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(video.videoWidth * scale);
  canvas.height = Math.round(video.videoHeight * scale);
  const context = canvas.getContext("2d");
  if (!context || !canvas.width) return Promise.resolve(null);
  if (mirror) {
    context.translate(canvas.width, 0);
    context.scale(-1, 1);
  }
  context.drawImage(video, 0, 0, canvas.width, canvas.height);
  return new Promise((resolve) => canvas.toBlob(resolve, "image/jpeg", 0.9));
}
