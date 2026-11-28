import { Camera, Mic, MicOff, MonitorUp, Radio, Square, VideoOff } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { SelectField } from "#/components/ui/select";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { streamClient } from "../api";
import { type MediaSession, MediaSessionError, publishOverWhip } from "../player/webrtc";

type Source = "camera" | "screen";
type Phase = "idle" | "connecting" | "publishing";
/** Why there is no picture; translated where it is shown, so the capture never depends on `t`. */
type MediaError = "" | "publish.denied" | "publish.noDevice" | "publish.captureFailed";

const DEFAULT_DEVICE = "default";

/**
 * Going live straight from the browser: pick the camera or a screen, pick
 * the microphone, check the preview, publish. The picture and sound travel
 * to the media server over WebRTC (WHIP) with the stream key, which is
 * fetched when publishing starts and kept nowhere.
 */
export function BrowserPublisher({
  streamId,
  onPublishingChange,
}: {
  streamId: string;
  onPublishingChange?(publishing: boolean): void;
}) {
  const { t } = useTranslation("live");
  const preview = useRef<HTMLVideoElement>(null);
  const [source, setSource] = useState<Source>("camera");
  const [cameraId, setCameraId] = useState(DEFAULT_DEVICE);
  const [microphoneId, setMicrophoneId] = useState(DEFAULT_DEVICE);
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
  const [media, setMedia] = useState<MediaStream | null>(null);
  const [mediaError, setMediaError] = useState<MediaError>("");
  const [micMuted, setMicMuted] = useState(false);
  const [phase, setPhase] = useState<Phase>("idle");
  const session = useRef<MediaSession | null>(null);
  const abort = useRef<AbortController | null>(null);
  const notify = useRef(onPublishingChange);
  useEffect(() => {
    notify.current = onPublishingChange;
  });

  const supported =
    typeof navigator !== "undefined" &&
    Boolean(navigator.mediaDevices?.getUserMedia) &&
    typeof RTCPeerConnection !== "undefined";

  // The capture follows the chosen source and devices, and nothing else: opening it again
  // stops the tracks a running broadcast sends. A screen is asked for with a tap (the
  // browser demands a gesture), the camera as soon as the panel opens.
  const [screenAsked, setScreenAsked] = useState(0);
  useEffect(() => {
    if (!supported) return;
    if (source === "screen" && screenAsked === 0) return;
    let cancelled = false;
    let opened: MediaStream | null = null;
    const audio = microphoneId === DEFAULT_DEVICE ? true : { deviceId: { exact: microphoneId } };
    const open = async () => {
      if (source === "camera")
        return navigator.mediaDevices.getUserMedia({
          video:
            cameraId === DEFAULT_DEVICE
              ? { width: { ideal: 1280 }, height: { ideal: 720 } }
              : { deviceId: { exact: cameraId }, width: { ideal: 1280 }, height: { ideal: 720 } },
          audio,
        });
      // The screen's picture (and its sound when the browser offers it) with the microphone on top.
      const screen = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: true });
      if (screen.getAudioTracks().length === 0) {
        try {
          const voice = await navigator.mediaDevices.getUserMedia({ audio });
          for (const track of voice.getAudioTracks()) screen.addTrack(track);
        } catch {
          // No microphone: the screen goes out silent.
        }
      }
      return screen;
    };
    open()
      .then(async (stream) => {
        if (cancelled) {
          for (const track of stream.getTracks()) track.stop();
          return;
        }
        opened = stream;
        setMedia(stream);
        setMediaError("");
        // Device names are only readable once access was granted.
        setDevices(await navigator.mediaDevices.enumerateDevices());
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setMedia(null);
        const name = error instanceof DOMException ? error.name : "";
        setMediaError(
          name === "NotAllowedError"
            ? "publish.denied"
            : name === "NotFoundError" || name === "OverconstrainedError"
              ? "publish.noDevice"
              : "publish.captureFailed",
        );
        if (source === "screen") setScreenAsked(0);
      });
    return () => {
      cancelled = true;
      if (opened) for (const track of opened.getTracks()) track.stop();
    };
  }, [supported, source, cameraId, microphoneId, screenAsked]);

  useEffect(() => {
    if (preview.current) preview.current.srcObject = media;
  }, [media]);

  useEffect(() => {
    if (media) for (const track of media.getAudioTracks()) track.enabled = !micMuted;
  }, [media, micMuted]);

  const stop = () => {
    abort.current?.abort();
    abort.current = null;
    session.current?.close();
    session.current = null;
    setPhase("idle");
    notify.current?.(false);
  };
  // Leaving the page ends the broadcast from this browser.
  useEffect(
    () => () => {
      abort.current?.abort();
      session.current?.close();
    },
    [],
  );

  // Changing the source while on air would need a new session: publishing stops with the capture.
  useEffect(() => {
    if (!media && session.current) stop();
  }, [media]);

  const start = async () => {
    if (!media) return;
    const controller = new AbortController();
    abort.current = controller;
    setPhase("connecting");
    try {
      // The key is asked for at the moment it is needed and handed straight to the handshake.
      const { ingest } = await streamClient().getIngestInfo({ streamId });
      if (!ingest) throw new MediaSessionError(0);
      const opened = await publishOverWhip(
        ingest.whipUrl,
        ingest.whipBearerToken,
        media,
        controller.signal,
      );
      if (controller.signal.aborted) return opened.close();
      session.current = opened;
      setPhase("publishing");
      notify.current?.(true);
      opened.connection.addEventListener("connectionstatechange", () => {
        const state = opened.connection.connectionState;
        if (session.current !== opened) return;
        if (state === "failed" || state === "closed") {
          toast.error(t("publish.lost"));
          stop();
        }
      });
    } catch (error) {
      if (controller.signal.aborted) return;
      setPhase("idle");
      toast.error(
        error instanceof MediaSessionError
          ? t(error.status === 0 ? "publish.unreachable" : "publish.refused")
          : errorMessage(error, { namespace: "live" }),
      );
    }
  };

  if (!supported)
    return (
      <p className="rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">
        {t("publish.unsupported")}
      </p>
    );

  const options = (kind: MediaDeviceKind, fallback: string) => [
    { value: DEFAULT_DEVICE, label: fallback },
    ...devices
      .filter((device) => device.kind === kind && device.deviceId && device.deviceId !== "default")
      .map((device, index) => ({
        value: device.deviceId,
        label: device.label || `${fallback} ${index + 1}`,
      })),
  ];
  const busy = phase !== "idle";

  return (
    <div className="flex flex-col gap-4" data-testid="browser-publisher" data-phase={phase}>
      <Tabs
        items={[
          { value: "camera", label: t("publish.camera") },
          { value: "screen", label: t("publish.screen") },
        ]}
        value={source}
        onChange={(next) => {
          if (busy) return toast.message(t("publish.stopFirst"));
          setMedia(null);
          setScreenAsked(0);
          setSource(next);
        }}
      />
      <div className="relative aspect-video w-full overflow-hidden rounded-2xl bg-black">
        <video
          ref={preview}
          className={cn("size-full object-contain", source === "camera" && "-scale-x-100")}
          autoPlay
          playsInline
          muted
          data-testid="publish-preview"
        />
        {!media && (
          <div className="absolute inset-0 grid place-items-center p-4 text-center text-white">
            {source === "screen" && !mediaError ? (
              <Button
                variant="secondary"
                icon={<MonitorUp className="size-4" />}
                onClick={() => setScreenAsked((value) => value + 1)}
              >
                {t("publish.chooseScreen")}
              </Button>
            ) : (
              <div className="flex max-w-xs flex-col items-center gap-2">
                <VideoOff className="size-8" />
                <p className="text-sm font-bold">{t(mediaError || "publish.opening")}</p>
                {source === "screen" && (
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => setScreenAsked((value) => value + 1)}
                  >
                    {t("publish.chooseScreen")}
                  </Button>
                )}
              </div>
            )}
          </div>
        )}
        {phase === "publishing" && (
          <span className="absolute top-2 left-2 flex h-6 items-center gap-1 rounded-md bg-live px-1.5 text-[11px] font-black text-white">
            <Radio className="size-3.5" />
            {t("publish.sending")}
          </span>
        )}
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        {source === "camera" && (
          <SelectField
            label={t("publish.cameraDevice")}
            value={cameraId}
            onChange={setCameraId}
            disabled={busy}
            options={options("videoinput", t("publish.defaultCamera"))}
          />
        )}
        <SelectField
          label={t("publish.microphone")}
          value={microphoneId}
          onChange={setMicrophoneId}
          disabled={busy}
          options={options("audioinput", t("publish.defaultMicrophone"))}
        />
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {phase === "publishing" ? (
          <Button variant="danger" icon={<Square className="size-4 fill-current" />} onClick={stop}>
            {t("publish.stop")}
          </Button>
        ) : (
          <Button
            loading={phase === "connecting"}
            disabled={!media}
            icon={
              source === "camera" ? <Camera className="size-4" /> : <MonitorUp className="size-4" />
            }
            onClick={() => void start()}
          >
            {t("publish.start")}
          </Button>
        )}
        <Button
          variant="secondary"
          aria-pressed={micMuted}
          disabled={!media}
          icon={micMuted ? <MicOff className="size-4" /> : <Mic className="size-4" />}
          onClick={() => setMicMuted(!micMuted)}
        >
          {t(micMuted ? "publish.unmuteMic" : "publish.muteMic")}
        </Button>
      </div>
      <p className="text-xs leading-relaxed text-ink-faint">{t("publish.keepOpen")}</p>
    </div>
  );
}
