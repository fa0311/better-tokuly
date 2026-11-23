import type { live } from "@tokuly/api";
import type Hls from "hls.js";
import {
  Maximize,
  Minimize,
  Pause,
  Play,
  Radio,
  Rewind,
  Volume2,
  VolumeX,
  Zap,
} from "lucide-react";
import { type ReactNode, type Ref, useEffect, useImperativeHandle, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/cn";
import { type MediaSession, watchOverWhep } from "./webrtc";

/** What other code can ask of a mounted live player. */
export interface LivePlayerHandle {
  /** Seconds since the stream went on air at the picture now on screen (the media timeline of chat and clips). */
  offsetSeconds(): number;
}

export interface LivePlayerProps {
  /** The latest playback URLs. A session that is playing keeps the ones it opened with. */
  playback: live.Playback;
  /** When the stream went on air (epoch milliseconds). */
  startedAtMs: number;
  /** The streamer's hint: start with WebRTC (sub-second) instead of HLS. */
  lowLatency: boolean;
  title?: string;
  poster?: string;
  /** Starts without sound (the streamer's own monitor). */
  muted?: boolean;
  /** The media session ended without being asked to: the page should read the stream again. */
  onSessionLost?(): void;
  ref?: Ref<LivePlayerHandle>;
  className?: string;
}

type Transport = "webrtc" | "hls";

/** Past this many seconds behind, the slider means "rewind" rather than "live". */
const LIVE_EDGE_SECONDS = 4;
const RETRY_MS = 2000;

/**
 * Plays a stream that is on air: WebRTC (WHEP) for sub-second latency where
 * the streamer asked for it, Low-Latency HLS otherwise or when WebRTC does
 * not connect, and the recording (DVR) for rewinding. Shows how far behind
 * the picture is and brings the viewer back to the live edge.
 */
export function LivePlayer({
  playback,
  startedAtMs,
  lowLatency,
  title,
  poster,
  muted: startMuted = false,
  onSessionLost,
  ref,
  className,
}: LivePlayerProps) {
  const { t } = useTranslation("live");
  const frame = useRef<HTMLDivElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const latest = useRef({ playback, onSessionLost });
  useEffect(() => {
    latest.current = { playback, onSessionLost };
  });

  const canWebrtc = Boolean(playback.whepUrl) && typeof RTCPeerConnection !== "undefined";
  const [transport, setTransport] = useState<Transport>(lowLatency && canWebrtc ? "webrtc" : "hls");
  /** Rewinding: the wall-clock moment the recording is played from; `null` at the live edge. */
  const [rewindFromMs, setRewindFromMs] = useState<number | null>(null);
  /** Changing it opens a new media session (a retry after the last one broke). */
  const [attempt, setAttempt] = useState(0);
  const [waiting, setWaiting] = useState(true);
  const [paused, setPaused] = useState(false);
  const [muted, setMuted] = useState(startMuted);
  /** The browser refused to start with sound: it plays muted until the viewer taps. */
  const [soundBlocked, setSoundBlocked] = useState(false);
  const [latency, setLatency] = useState<number | null>(null);
  const [fullscreen, setFullscreen] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  /** The slider while it is being dragged (seconds behind live). */
  const [dragging, setDragging] = useState<number | null>(null);

  const dvrSeconds = playback.dvrGetUrl
    ? Math.max(0, Math.min(playback.dvrWindowSeconds, Math.floor((now - startedAtMs) / 1000)))
    : 0;

  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);

  // The media session. `attempt`, the transport and rewinding decide when a new one opens;
  // fresh playback URLs alone do not (the one that plays keeps working).
  useEffect(() => {
    const element = video.current;
    if (!element) return;
    const abort = new AbortController();
    let session: MediaSession | null = null;
    let hls: Hls | null = null;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let latencyTimer: ReturnType<typeof setInterval> | undefined;
    const urls = latest.current.playback;
    setWaiting(true);
    setLatency(null);

    const start = () => {
      element.play().catch(() => {
        if (abort.signal.aborted || element.muted) return;
        // Autoplay with sound was refused: start silent and offer the sound.
        element.muted = true;
        setMuted(true);
        setSoundBlocked(true);
        void element.play().catch(() => {});
      });
    };
    const again = (lost: boolean) => {
      if (abort.signal.aborted) return;
      if (lost) latest.current.onSessionLost?.();
      retry = setTimeout(() => setAttempt((value) => value + 1), RETRY_MS);
    };

    if (rewindFromMs !== null) {
      const seconds = Math.max(1, Math.ceil((Date.now() - rewindFromMs) / 1000));
      element.src = `${urls.dvrGetUrl}&start=${encodeURIComponent(
        new Date(rewindFromMs).toISOString(),
      )}&duration=${seconds}`;
      const onEnded = () => setRewindFromMs(null);
      const onError = () => setRewindFromMs(null);
      element.addEventListener("ended", onEnded);
      element.addEventListener("error", onError);
      start();
      return () => {
        abort.abort();
        element.removeEventListener("ended", onEnded);
        element.removeEventListener("error", onError);
        element.removeAttribute("src");
        element.load();
      };
    }

    if (transport === "webrtc") {
      watchOverWhep(
        urls.whepUrl,
        urls.token,
        (media) => {
          if (element.srcObject !== media) element.srcObject = media;
          start();
        },
        abort.signal,
      )
        .then((opened) => {
          if (abort.signal.aborted) return opened.close();
          session = opened;
          setLatency(0);
          opened.connection.addEventListener("connectionstatechange", () => {
            const state = opened.connection.connectionState;
            if (state === "failed" || state === "closed" || state === "disconnected") again(true);
          });
        })
        .catch(() => {
          if (abort.signal.aborted) return;
          // No WebRTC path to the server (a strict network): HLS always gets through.
          if (urls.hlsUrl) setTransport("hls");
          else again(true);
        });
    } else {
      void import("hls.js").then(({ default: HlsPlayer }) => {
        if (abort.signal.aborted) return;
        if (!HlsPlayer.isSupported()) {
          // Browsers that play HLS themselves (iPhones).
          element.src = urls.hlsUrl;
          element.addEventListener("error", () => again(true), { once: true });
          start();
          return;
        }
        const player = new HlsPlayer({ lowLatencyMode: true, backBufferLength: 30 });
        hls = player;
        player.on(HlsPlayer.Events.MANIFEST_PARSED, start);
        player.on(HlsPlayer.Events.ERROR, (_event, data) => {
          if (!data.fatal) return;
          player.destroy();
          hls = null;
          // The playlist of a stream that just went on air takes a moment to exist.
          again(data.type !== HlsPlayer.ErrorTypes.NETWORK_ERROR);
        });
        player.loadSource(urls.hlsUrl);
        player.attachMedia(element);
        latencyTimer = setInterval(() => {
          if (Number.isFinite(player.latency) && player.latency > 0) setLatency(player.latency);
        }, 1000);
      });
    }

    return () => {
      abort.abort();
      clearTimeout(retry);
      clearInterval(latencyTimer);
      session?.close();
      hls?.destroy();
      element.srcObject = null;
      element.removeAttribute("src");
      element.load();
    };
  }, [transport, rewindFromMs, attempt]);

  useEffect(() => {
    const element = video.current;
    if (!element) return;
    const onPlaying = () => {
      setWaiting(false);
      setPaused(false);
    };
    const onWaiting = () => setWaiting(true);
    const onPause = () => setPaused(true);
    const onVolume = () => setMuted(element.muted);
    element.addEventListener("playing", onPlaying);
    element.addEventListener("waiting", onWaiting);
    element.addEventListener("pause", onPause);
    element.addEventListener("volumechange", onVolume);
    const onFullscreen = () => setFullscreen(document.fullscreenElement === frame.current);
    document.addEventListener("fullscreenchange", onFullscreen);
    return () => {
      element.removeEventListener("playing", onPlaying);
      element.removeEventListener("waiting", onWaiting);
      element.removeEventListener("pause", onPause);
      element.removeEventListener("volumechange", onVolume);
      document.removeEventListener("fullscreenchange", onFullscreen);
    };
  }, []);

  /** Seconds the picture on screen lies behind the live edge. */
  const behind = () => {
    const element = video.current;
    if (rewindFromMs !== null)
      return Math.max(0, (Date.now() - rewindFromMs) / 1000 - (element?.currentTime ?? 0));
    return latency ?? 0;
  };
  useImperativeHandle(ref, () => ({
    offsetSeconds: () => Math.max(0, (Date.now() - startedAtMs) / 1000 - behind()),
  }));

  const rewindTo = (secondsBehind: number) => {
    setPaused(false);
    if (secondsBehind < LIVE_EDGE_SECONDS) setRewindFromMs(null);
    else setRewindFromMs(Math.max(startedAtMs, Date.now() - secondsBehind * 1000));
  };
  const toggle = () => {
    const element = video.current;
    if (!element) return;
    if (!element.paused) return element.pause();
    // A live picture that was paused is stale: come back at the live edge.
    if (rewindFromMs === null) setAttempt((value) => value + 1);
    else void element.play().catch(() => {});
  };
  const toggleSound = () => {
    const element = video.current;
    if (!element) return;
    element.muted = !element.muted;
    setSoundBlocked(false);
  };
  const toggleFullscreen = () => {
    if (document.fullscreenElement) void document.exitFullscreen();
    else void frame.current?.requestFullscreen?.().catch(() => {});
  };

  const atLive = rewindFromMs === null;
  const shownBehind = dragging ?? Math.round(behind());
  const latencyText = !atLive
    ? t("player.behind", { clock: clock(shownBehind) })
    : transport === "webrtc"
      ? t("player.latencyRealtime")
      : latency === null
        ? t("player.latencyUnknown")
        : t("player.latencySeconds", { count: Math.max(1, Math.round(latency)) });

  return (
    <div
      ref={frame}
      className={cn(
        "group relative aspect-video w-full overflow-hidden bg-black text-white",
        !fullscreen && "md:rounded-2xl",
        className,
      )}
      data-testid="live-player"
      data-transport={atLive ? transport : "dvr"}
      data-playing={!waiting && !paused}
    >
      <video
        ref={video}
        className="size-full object-contain"
        playsInline
        autoPlay
        muted={muted}
        poster={poster}
        aria-label={title}
        onClick={toggle}
      />
      {waiting && !paused && (
        <div className="pointer-events-none absolute inset-0 grid place-items-center">
          <Spinner className="size-10 text-white" />
        </div>
      )}
      {soundBlocked && (
        <button
          type="button"
          onClick={toggleSound}
          className="pressable absolute top-3 left-1/2 flex h-10 -translate-x-1/2 items-center gap-2 rounded-full bg-white px-4 text-sm font-black text-[#17122b]"
        >
          <VolumeX className="size-4" />
          {t("player.unmute")}
        </button>
      )}

      <div className="absolute inset-x-0 bottom-0 flex flex-col gap-1 bg-gradient-to-t from-black/75 to-transparent px-3 pt-8 pb-2 transition-opacity focus-within:opacity-100 group-hover:opacity-100 md:opacity-0 [@media(hover:none)]:opacity-100">
        {dvrSeconds >= 10 && (
          <input
            type="range"
            aria-label={t("player.rewindSlider")}
            aria-valuetext={
              shownBehind < LIVE_EDGE_SECONDS
                ? t("player.live")
                : t("player.behind", { clock: clock(shownBehind) })
            }
            min={0}
            max={dvrSeconds}
            step={1}
            value={dvrSeconds - Math.min(dvrSeconds, shownBehind)}
            onChange={(event) => setDragging(dvrSeconds - Number(event.target.value))}
            onPointerUp={() => {
              if (dragging !== null) rewindTo(dragging);
              setDragging(null);
            }}
            onKeyUp={() => {
              if (dragging !== null) rewindTo(dragging);
              setDragging(null);
            }}
            className="h-1.5 w-full cursor-pointer accent-[var(--live)]"
          />
        )}
        <div className="flex items-center gap-1">
          <ControlButton label={paused ? t("player.play") : t("player.pause")} onClick={toggle}>
            {paused ? (
              <Play className="size-5 fill-current" />
            ) : (
              <Pause className="size-5 fill-current" />
            )}
          </ControlButton>
          <ControlButton
            label={muted ? t("player.soundOn") : t("player.soundOff")}
            onClick={toggleSound}
          >
            {muted ? <VolumeX className="size-5" /> : <Volume2 className="size-5" />}
          </ControlButton>
          {dvrSeconds >= 10 && (
            <ControlButton
              label={t("player.back10")}
              onClick={() => rewindTo(Math.min(dvrSeconds, behind() + 10))}
            >
              <Rewind className="size-5" />
            </ControlButton>
          )}
          {atLive ? (
            <span className="ml-1 flex h-7 items-center gap-1.5 rounded-full bg-live px-2.5 text-xs font-black tracking-wide">
              <Radio className="size-3.5" />
              {t("player.live")}
            </span>
          ) : (
            <button
              type="button"
              onClick={() => rewindTo(0)}
              className="pressable ml-1 flex h-7 items-center gap-1.5 rounded-full bg-white px-2.5 text-xs font-black text-[#17122b]"
            >
              <Radio className="size-3.5" />
              {t("player.jumpToLive")}
            </button>
          )}
          <span
            className="ml-1 min-w-0 truncate text-xs font-bold text-white/85"
            data-testid="live-latency"
          >
            {latencyText}
          </span>
          <span className="flex-1" />
          {atLive && canWebrtc && Boolean(playback.hlsUrl) && (
            <button
              type="button"
              aria-pressed={transport === "webrtc"}
              aria-label={t("player.lowLatencyToggle")}
              title={t("player.lowLatencyToggle")}
              onClick={() => setTransport(transport === "webrtc" ? "hls" : "webrtc")}
              className={cn(
                "flex h-8 shrink-0 items-center gap-1 rounded-full px-2.5 text-xs font-black whitespace-nowrap",
                transport === "webrtc" ? "bg-lime text-[#17122b]" : "bg-white/15 hover:bg-white/25",
              )}
            >
              <Zap className="size-3.5" />
              <span className="max-sm:sr-only">{t("player.lowLatency")}</span>
            </button>
          )}
          <ControlButton
            label={fullscreen ? t("player.exitFullscreen") : t("player.fullscreen")}
            onClick={toggleFullscreen}
          >
            {fullscreen ? <Minimize className="size-5" /> : <Maximize className="size-5" />}
          </ControlButton>
        </div>
      </div>
    </div>
  );
}

function ControlButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-white/20"
    >
      {children}
    </button>
  );
}

/** `m:ss` (or `h:mm:ss`) for a number of seconds. */
export function clock(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}
