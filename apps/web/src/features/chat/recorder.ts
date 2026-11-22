import { peaksOf } from "./waveform";

/** Recording in the browser with `MediaRecorder`: voice messages and short videos. */

const AUDIO_TYPES = ["audio/webm;codecs=opus", "audio/webm", "audio/mp4", "audio/ogg;codecs=opus"];
const VIDEO_TYPES = [
  "video/webm;codecs=vp9,opus",
  "video/webm;codecs=vp8,opus",
  "video/webm",
  "video/mp4",
];

/** The first container this browser can record, or undefined to let it choose. */
export function recordingType(kind: "audio" | "video"): string | undefined {
  if (typeof MediaRecorder === "undefined") return undefined;
  return (kind === "audio" ? AUDIO_TYPES : VIDEO_TYPES).find((type) =>
    MediaRecorder.isTypeSupported(type),
  );
}

/** The file extension that goes with a recorded type. */
export function extensionOf(type: string): string {
  if (type.includes("mp4")) return type.startsWith("audio") ? "m4a" : "mp4";
  if (type.includes("ogg")) return "ogg";
  return "webm";
}

export function canRecord(): boolean {
  return typeof MediaRecorder !== "undefined" && Boolean(navigator.mediaDevices?.getUserMedia);
}

export interface VoiceRecording {
  blob: Blob;
  fileName: string;
  durationSeconds: number;
  /** Peaks (0-255) for the bubble shown while the message is on its way. */
  waveform: Uint8Array;
}

/**
 * One voice recording in progress. `onLevel` is told the loudness (0..1)
 * about fifteen times a second, for the live waveform.
 */
export class VoiceRecorder {
  private readonly chunks: Blob[] = [];
  private readonly levels: number[] = [];
  private readonly startedAt = performance.now();
  private meter: ReturnType<typeof setInterval> | undefined;
  private context: AudioContext | undefined;

  private constructor(
    private readonly stream: MediaStream,
    private readonly recorder: MediaRecorder,
  ) {}

  /** Asks for the microphone and starts recording. Rejects when it is refused or missing. */
  static async start(onLevel: (level: number) => void): Promise<VoiceRecorder> {
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { echoCancellation: true, noiseSuppression: true },
    });
    const type = recordingType("audio");
    const recorder = new MediaRecorder(stream, type ? { mimeType: type } : undefined);
    const session = new VoiceRecorder(stream, recorder);
    recorder.ondataavailable = (event) => {
      if (event.data.size > 0) session.chunks.push(event.data);
    };
    session.watchLevel(onLevel);
    recorder.start(250);
    return session;
  }

  private watchLevel(onLevel: (level: number) => void): void {
    try {
      const context = new AudioContext();
      const analyser = context.createAnalyser();
      analyser.fftSize = 512;
      context.createMediaStreamSource(this.stream).connect(analyser);
      const samples = new Uint8Array(analyser.fftSize);
      this.context = context;
      this.meter = setInterval(() => {
        analyser.getByteTimeDomainData(samples);
        let sum = 0;
        for (const sample of samples) sum += ((sample - 128) / 128) ** 2;
        // Speech sits low on a linear scale: lift it so quiet talking still shows.
        const level = Math.min(1, Math.sqrt(sum / samples.length) * 3.2);
        this.levels.push(level);
        onLevel(level);
      }, 66);
    } catch {
      // No Web Audio: the recording works, only without a live waveform.
    }
  }

  get elapsedSeconds(): number {
    return (performance.now() - this.startedAt) / 1000;
  }

  private release(): void {
    if (this.meter) clearInterval(this.meter);
    void this.context?.close().catch(() => {});
    for (const track of this.stream.getTracks()) track.stop();
  }

  /** Ends the recording and hands over what was recorded. */
  stop(): Promise<VoiceRecording> {
    const durationSeconds = this.elapsedSeconds;
    return new Promise((resolve) => {
      this.recorder.onstop = () => {
        const type = this.recorder.mimeType || this.chunks[0]?.type || "audio/webm";
        this.release();
        resolve({
          blob: new Blob(this.chunks, { type }),
          fileName: `voice-${Date.now()}.${extensionOf(type)}`,
          durationSeconds,
          waveform: peaksOf(this.levels),
        });
      };
      if (this.recorder.state === "inactive") this.recorder.onstop?.(new Event("stop"));
      else this.recorder.stop();
    });
  }

  /** Throws the recording away. */
  cancel(): void {
    this.recorder.onstop = null;
    if (this.recorder.state !== "inactive") this.recorder.stop();
    this.release();
  }
}
