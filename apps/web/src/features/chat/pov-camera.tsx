import { useQueryClient } from "@tanstack/react-query";
import { Circle, ImagePlus, RefreshCcw, Send, Square, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { describeFile, type OutgoingFile, releaseFiles } from "./attachments";
import { useConversationContext } from "./conversation-context";
import { send } from "./outbox";
import { extensionOf, recordingType } from "./recorder";
import { playTime } from "./text";

/** A POV answer holds at most this many photos and videos. */
const MAX_SHOTS = 4;
/** A POV video is a glimpse, not a film. */
const MAX_VIDEO_SECONDS = 15;

/**
 * Answers a POV request with the camera: a photo or a short video taken on
 * the spot. Where the camera cannot be used (no permission, no camera) the
 * device's own camera or file picker stands in.
 */
export function PovCamera({ open, onClose, requestId }: { open: boolean; onClose(): void; requestId: string }) {
  const { t } = useTranslation("chat");
  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()} title={t("pov.cameraTitle")} description={t("pov.cameraLead")}>
      {open && <Camera requestId={requestId} onDone={onClose} />}
    </Dialog>
  );
}

function Camera({ requestId, onDone }: { requestId: string; onDone(): void }) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const { conversation } = useConversationContext();
  const preview = useRef<HTMLVideoElement>(null);
  const fallback = useRef<HTMLInputElement>(null);
  const recorder = useRef<MediaRecorder | null>(null);
  const kept = useRef(false);
  const [stream, setStream] = useState<MediaStream | null>(null);
  const [unavailable, setUnavailable] = useState(!navigator.mediaDevices?.getUserMedia);
  const [facing, setFacing] = useState<"environment" | "user">("environment");
  const [mode, setMode] = useState<"photo" | "video">("photo");
  const [recordingSince, setRecordingSince] = useState<number | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [shots, setShots] = useState<OutgoingFile[]>([]);
  const [caption, setCaption] = useState("");
  const full = shots.length >= MAX_SHOTS;

  // The camera runs while the dialog is open, and is let go with it.
  useEffect(() => {
    if (unavailable) return;
    let cancelled = false;
    let current: MediaStream | null = null;
    const start = async () => {
      try {
        current = await navigator.mediaDevices.getUserMedia({ video: { facingMode: facing }, audio: true });
      } catch {
        // No microphone (or it was refused): photos and silent videos still work.
        current = await navigator.mediaDevices.getUserMedia({ video: { facingMode: facing } });
      }
      if (cancelled) {
        for (const track of current.getTracks()) track.stop();
        return;
      }
      setStream(current);
      if (preview.current) preview.current.srcObject = current;
    };
    start().catch(() => !cancelled && setUnavailable(true));
    return () => {
      cancelled = true;
      for (const track of current?.getTracks() ?? []) track.stop();
      setStream(null);
    };
  }, [facing, unavailable]);

  // Shots that were not sent are released when the dialog closes.
  const shotsRef = useRef(shots);
  useEffect(() => {
    shotsRef.current = shots;
  }, [shots]);
  useEffect(
    () => () => {
      if (!kept.current) releaseFiles(shotsRef.current);
    },
    [],
  );

  useEffect(() => {
    if (recordingSince === null) return;
    const timer = setInterval(() => {
      const seconds = (Date.now() - recordingSince) / 1000;
      setElapsed(seconds);
      if (seconds >= MAX_VIDEO_SECONDS) recorder.current?.stop();
    }, 200);
    return () => clearInterval(timer);
  }, [recordingSince]);

  const add = async (file: Blob, name: string, kind?: "image" | "video") => {
    const described = await describeFile(file, { name, kind });
    setShots((current) => (current.length >= MAX_SHOTS ? current : [...current, described]));
  };

  const takePhoto = () => {
    const video = preview.current;
    if (!video?.videoWidth) return;
    const canvas = document.createElement("canvas");
    canvas.width = video.videoWidth;
    canvas.height = video.videoHeight;
    canvas.getContext("2d")?.drawImage(video, 0, 0);
    canvas.toBlob((blob) => blob && void add(blob, `pov-${Date.now()}.jpg`, "image"), "image/jpeg", 0.9);
  };

  const startVideo = () => {
    if (!stream) return;
    const type = recordingType("video");
    const chunks: Blob[] = [];
    const session = new MediaRecorder(stream, type ? { mimeType: type } : undefined);
    session.ondataavailable = (event) => event.data.size > 0 && chunks.push(event.data);
    session.onstop = () => {
      recorder.current = null;
      setRecordingSince(null);
      setElapsed(0);
      const recorded = session.mimeType || type || "video/webm";
      if (chunks.length > 0) void add(new Blob(chunks, { type: recorded }), `pov-${Date.now()}.${extensionOf(recorded)}`, "video");
    };
    recorder.current = session;
    session.start(250);
    setRecordingSince(Date.now());
  };

  const choose = async (files: FileList | null) => {
    for (const file of Array.from(files ?? []).slice(0, MAX_SHOTS - shots.length)) {
      if (!/^(image|video)\//.test(file.type)) {
        toast.error(t("pov.onlyMedia"));
        continue;
      }
      await add(file, file.name);
    }
    if (fallback.current) fallback.current.value = "";
  };

  const submit = () => {
    if (shots.length === 0) return;
    kept.current = true;
    send(queryClient, conversation.id, { text: caption.trim(), files: shots, povRequestId: requestId });
    onDone();
  };

  const recording = recordingSince !== null;
  return (
    <div className="flex flex-col gap-3">
      {unavailable ? (
        <div className="flex flex-col items-center gap-3 rounded-card bg-surface-2 p-6 text-center">
          <p className="text-5xl">📸</p>
          <p className="text-sm text-ink-soft">{t("pov.cameraUnavailable")}</p>
          <Button variant="secondary" icon={<ImagePlus className="size-4" />} disabled={full} onClick={() => fallback.current?.click()}>
            {t("pov.openCamera")}
          </Button>
        </div>
      ) : (
        <>
          <div className="relative overflow-hidden rounded-card bg-black">
            <video ref={preview} autoPlay muted playsInline className={cn("aspect-[3/4] w-full object-cover sm:aspect-[4/3]", facing === "user" && "-scale-x-100")} data-testid="pov-preview" />
            {recording && (
              <span className="absolute top-3 left-1/2 flex -translate-x-1/2 items-center gap-1.5 rounded-full bg-live px-3 py-1 text-xs font-black text-white tabular-nums">
                <span className="size-2 animate-pulse rounded-full bg-white" />
                {playTime(elapsed)} / {playTime(MAX_VIDEO_SECONDS)}
              </span>
            )}
            <button
              type="button"
              aria-label={t("pov.flip")}
              disabled={recording}
              onClick={() => setFacing((current) => (current === "user" ? "environment" : "user"))}
              className="absolute top-3 right-3 grid size-10 place-items-center rounded-full bg-black/50 text-white disabled:opacity-40"
            >
              <RefreshCcw className="size-5" />
            </button>
          </div>
          <div className="flex items-center justify-between gap-3">
            <Tabs<"photo" | "video">
              value={mode}
              onChange={(next) => !recording && setMode(next)}
              items={[
                { value: "photo", label: t("pov.photo") },
                { value: "video", label: t("pov.video") },
              ]}
            />
            <button
              type="button"
              disabled={!stream || (full && !recording)}
              aria-label={mode === "photo" ? t("pov.shoot") : recording ? t("pov.stop") : t("pov.record")}
              onClick={() => (mode === "photo" ? takePhoto() : recording ? recorder.current?.stop() : startVideo())}
              className={cn("pressable sticker grid size-16 shrink-0 place-items-center rounded-full disabled:opacity-40", mode === "video" ? "bg-live text-white" : "bg-surface text-ink")}
            >
              {recording ? <Square className="size-6 fill-current" /> : <Circle className={cn("size-9", mode === "video" && "fill-current")} />}
            </button>
            <button type="button" disabled={full || recording} onClick={() => fallback.current?.click()} className="grid size-11 place-items-center rounded-full text-ink-soft hover:bg-surface-2 disabled:opacity-40" aria-label={t("pov.pick")}>
              <ImagePlus className="size-5" />
            </button>
          </div>
        </>
      )}

      <input ref={fallback} type="file" accept="image/*,video/*" capture="environment" multiple className="hidden" data-testid="pov-file-input" onChange={(event) => void choose(event.target.files)} />

      {shots.length > 0 && (
        <>
          <ul className="flex gap-2" aria-label={t("pov.shots")}>
            {shots.map((shot) => (
              <li key={shot.id} className="relative size-20 overflow-hidden rounded-2xl bg-surface-3">
                {shot.kind === "video" ? <video src={`${shot.previewUrl}#t=0.1`} muted playsInline preload="metadata" className="size-full object-cover" /> : <img src={shot.previewUrl} alt="" className="size-full object-cover" />}
                <button
                  type="button"
                  aria-label={t("actions.remove")}
                  onClick={() => {
                    releaseFiles([shot]);
                    setShots((current) => current.filter((candidate) => candidate.id !== shot.id));
                  }}
                  className="absolute top-1 right-1 grid size-6 place-items-center rounded-full bg-black/60 text-white"
                >
                  <X className="size-4" />
                </button>
              </li>
            ))}
          </ul>
          <input
            value={caption}
            onChange={(event) => setCaption(event.target.value)}
            placeholder={t("pov.caption")}
            aria-label={t("pov.caption")}
            maxLength={500}
            className="h-11 rounded-control border-2 border-line bg-surface px-4 text-[15px] outline-none placeholder:text-ink-faint focus:border-brand"
          />
        </>
      )}
      <Button block variant="lime" disabled={shots.length === 0 || recording} icon={<Send className="size-4" />} onClick={submit}>
        {t("pov.send")}
      </Button>
    </div>
  );
}
