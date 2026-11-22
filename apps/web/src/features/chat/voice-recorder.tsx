import { Mic, Send, Trash2 } from "lucide-react";
import { type PointerEvent, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { cn } from "#/lib/cn";
import { describeFile, type OutgoingFile } from "./attachments";
import { VoiceRecorder } from "./recorder";
import { playTime } from "./text";

/** A voice message is cut off (and sent) at this length. */
const MAX_SECONDS = 300;
/** Holding the button longer than this makes it "hold to talk": letting go sends. */
const HOLD_MS = 450;
/** Recordings shorter than this are a slip of the finger, not a message. */
const MIN_SECONDS = 0.6;
/** Bars of the live waveform. */
const LIVE_BARS = 36;

/**
 * The microphone button of the composer and, while recording, the bar that
 * replaces the text field. Tap to start and tap send when done, or hold the
 * button while talking and let go to send (sliding away cancels).
 */
export function VoiceRecorderControl({ recording, onRecordingChange, onRecorded }: { recording: boolean; onRecordingChange(recording: boolean): void; onRecorded(file: OutgoingFile): void }) {
  const { t } = useTranslation("chat");
  const session = useRef<VoiceRecorder | null>(null);
  /** The request for the microphone that is under way; cancelled when nobody waits for it any more. */
  const attempt = useRef<{ cancelled: boolean } | null>(null);
  const [levels, setLevels] = useState<number[]>([]);
  const [elapsed, setElapsed] = useState(0);

  const finish = async (keep: boolean) => {
    const current = session.current;
    session.current = null;
    // Let go (or left) while the microphone was still being asked for: that recording must not start.
    if (attempt.current) attempt.current.cancelled = true;
    onRecordingChange(false);
    setLevels([]);
    setElapsed(0);
    if (!current) return;
    if (!keep || current.elapsedSeconds < MIN_SECONDS) {
      current.cancel();
      if (keep) toast(t("voice.tooShort"));
      return;
    }
    const recorded = await current.stop();
    onRecorded(await describeFile(recorded.blob, { name: recorded.fileName, kind: "audio", durationSeconds: recorded.durationSeconds, waveform: recorded.waveform }));
  };

  const start = async () => {
    if (session.current || attempt.current) return;
    const mine = { cancelled: false };
    attempt.current = mine;
    try {
      const started = await VoiceRecorder.start((level) => setLevels((current) => [...current.slice(-(LIVE_BARS - 1)), level]));
      // Nobody is waiting for it any more: free the microphone at once.
      if (mine.cancelled) {
        started.cancel();
        return;
      }
      session.current = started;
      onRecordingChange(true);
    } catch {
      if (!mine.cancelled) toast.error(t("voice.noMicrophone"));
    } finally {
      if (attempt.current === mine) attempt.current = null;
    }
  };

  // The timer, and the cut-off at the maximum length.
  useEffect(() => {
    if (!recording) return;
    const timer = setInterval(() => {
      const seconds = session.current?.elapsedSeconds ?? 0;
      setElapsed(seconds);
      if (seconds >= MAX_SECONDS) void finish(true);
    }, 200);
    return () => clearInterval(timer);
    // `finish` only reads refs and stable callbacks.
  }, [recording]);

  // Leaving the screen mid-recording throws the recording away and frees the microphone.
  useEffect(
    () => () => {
      if (attempt.current) attempt.current.cancelled = true;
      session.current?.cancel();
      session.current = null;
    },
    [],
  );

  // Pressing starts the recording. A quick tap leaves it running ("locked": the bar's buttons end it);
  // holding makes it "hold to talk", where letting go sends and sliding away cancels. The release is
  // watched on the window, because the button itself is replaced by the recording bar meanwhile.
  const onPointerDown = (event: PointerEvent<HTMLButtonElement>) => {
    if (recording || event.button !== 0) return;
    const press = { at: performance.now(), x: event.clientX, y: event.clientY };
    const release = (up: globalThis.PointerEvent) => {
      window.removeEventListener("pointerup", release);
      window.removeEventListener("pointercancel", release);
      if (performance.now() - press.at < HOLD_MS) return;
      const slidAway = up.type === "pointercancel" || Math.hypot(up.clientX - press.x, up.clientY - press.y) > 80;
      void finish(!slidAway);
    };
    window.addEventListener("pointerup", release);
    window.addEventListener("pointercancel", release);
    void start();
  };

  if (!recording) {
    return (
      <button
        type="button"
        aria-label={t("voice.record")}
        title={t("voice.hint")}
        onPointerDown={onPointerDown}
        // Keyboard and assistive technology: Enter / Space starts a locked recording.
        onKeyDown={(event) => (event.key === "Enter" || event.key === " ") && !event.repeat && void start()}
        className="pressable grid size-11 shrink-0 touch-none select-none place-items-center rounded-full bg-surface-2 text-ink"
      >
        <Mic className="size-5" />
      </button>
    );
  }

  const bars = [...Array.from({ length: Math.max(0, LIVE_BARS - levels.length) }, () => 0), ...levels];
  return (
    <div className="flex min-w-0 flex-1 items-center gap-2" data-testid="voice-recording">
      <button type="button" aria-label={t("voice.cancel")} onClick={() => void finish(false)} className="grid size-11 shrink-0 place-items-center rounded-full text-danger hover:bg-surface-2">
        <Trash2 className="size-5" />
      </button>
      <div className="flex h-11 min-w-0 flex-1 items-center gap-2 rounded-full bg-surface-2 px-4" role="status" aria-label={t("voice.recording", { time: playTime(elapsed) })}>
        <span className="size-2.5 shrink-0 animate-pulse rounded-full bg-live" />
        <span className="shrink-0 text-sm font-black tabular-nums">{playTime(elapsed)}</span>
        <span className="flex h-7 min-w-0 flex-1 items-center justify-end gap-[2px] overflow-hidden" aria-hidden>
          {bars.map((level, index) => (
            <span key={index} className={cn("w-[3px] shrink-0 rounded-full bg-brand transition-[height] duration-75")} style={{ height: `${Math.max(10, Math.round(level * 100))}%` }} />
          ))}
        </span>
      </div>
      <button
        type="button"
        aria-label={t("voice.send")}
        // The tap that started the recording may end on this button, which appears under the finger: that is not "send".
        onClick={() => (session.current?.elapsedSeconds ?? 1) > 0.4 && void finish(true)}
        className="pressable sticker-sm grid size-11 shrink-0 place-items-center rounded-full bg-brand text-on-brand"
      >
        <Send className="size-5" />
      </button>
    </div>
  );
}
