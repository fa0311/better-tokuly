import { useQuery } from "@tanstack/react-query";
import type { media, video } from "@tokuly/api";
import { chapterAt } from "@tokuly/core/video";
import {
  Captions,
  CaptionsOff,
  FastForward,
  Lock,
  Maximize,
  Minimize,
  Pause,
  PictureInPicture2,
  Play,
  RectangleHorizontal,
  Rewind,
  RotateCcw,
  Settings,
  Volume1,
  Volume2,
  VolumeX,
} from "lucide-react";
import {
  type MouseEvent,
  type ReactNode,
  type PointerEvent as ReactPointerEvent,
  type Ref,
  type RefObject,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from "react";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { formatClock } from "../format";
import { useMediaEngine } from "./engine";
import { scriptUrl } from "./media-url";
import { Scrubber } from "./scrubber";
import { SettingsMenu } from "./settings-menu";
import { commandForKey, type PlayerCommand, playerOwnsKey, stepRate } from "./shortcuts";
import { activeCues, type Cue, parseVtt } from "./vtt";

/** What other code can ask of a mounted player (through its `ref`). */
export interface PlayerHandle {
  currentTime(): number;
  duration(): number;
  paused(): boolean;
  play(): Promise<void>;
  pause(): void;
  seek(seconds: number): void;
  setRate(rate: number): void;
  /** The `<video>` itself, for drawing a frame to a canvas. */
  element(): HTMLVideoElement | null;
}

export interface VideoPlayerProps {
  /** The processed upload: streams, poster, seek-preview track. */
  source: media.VideoInfo;
  /** Shown over the picture in fullscreen. */
  title?: string;
  chapters?: video.Chapter[];
  subtitles?: video.SubtitleTrack[];
  /** Where to start, in seconds (a resume position or a `?t=` link). */
  startAt?: number;
  autoPlay?: boolean;
  /**
   * Take the page's keyboard shortcuts (space, k, j, l, arrows, f, m, digits…).
   * One player per page should have them.
   */
  shortcuts?: boolean;
  theatre?: boolean;
  /** Given, the player offers a theatre-mode button (and the `t` key). */
  onTheatreChange?(theatre: boolean): void;
  /**
   * Someone else drives playback (a watch party's host): play, pause, seek
   * and speed are disabled and this text says why. Volume, quality, captions
   * and fullscreen stay with the viewer.
   */
  lockedReason?: string;
  ref?: Ref<PlayerHandle>;
  onPlay?(): void;
  onPause?(): void;
  /** A seek finished at this position. */
  onSeeked?(seconds: number): void;
  onEnded?(): void;
  onRateChange?(rate: number): void;
  /** Playback stalled for data, or resumed. */
  onWaiting?(waiting: boolean): void;
  /** Replaces the default "watch again" once playback has ended. */
  endScreen?: ReactNode;
  className?: string;
}

const CAPTIONS_KEY = "tokuly.player.captions";
/** Controls fade this long after the last movement while playing. */
const IDLE_MS = 2800;
const DOUBLE_TAP_MS = 320;

function storedCaptions(): string | null {
  try {
    return window.localStorage.getItem(CAPTIONS_KEY);
  } catch {
    return null;
  }
}

/** The track to show when the player opens: the viewer's last language, else the video's default track. */
function initialTrack(tracks: video.SubtitleTrack[]): string | null {
  const wanted = storedCaptions();
  if (wanted === "off") return null;
  const preferred = wanted ? tracks.find((track) => track.language === wanted) : undefined;
  return (preferred ?? tracks.find((track) => track.isDefault))?.id ?? null;
}

/**
 * The video player: adaptive HLS with its own controls — play and pause, a
 * scrub bar with chapter marks and frame previews, volume, speed, quality,
 * subtitles, picture-in-picture, fullscreen and theatre mode — driven by the
 * mouse, the keyboard, or a thumb (tap for the controls, double-tap the
 * sides to jump ten seconds).
 */
export function VideoPlayer({
  source,
  title,
  chapters = [],
  subtitles = [],
  startAt = 0,
  autoPlay = false,
  shortcuts = false,
  theatre = false,
  onTheatreChange,
  lockedReason,
  ref,
  onPlay,
  onPause,
  onSeeked,
  onEnded,
  onRateChange,
  onWaiting,
  endScreen,
  className,
}: VideoPlayerProps) {
  const { t } = useTranslation("video");
  const container = useRef<HTMLDivElement>(null);
  const element = useRef<HTMLVideoElement>(null);
  const [state, controls] = useMediaEngine(element, source, { startAt, autoPlay });
  const locked = Boolean(lockedReason);

  const [active, setActive] = useState(true);
  const [scrubbing, setScrubbing] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);
  const [pip, setPip] = useState(false);
  const [trackId, setTrackId] = useState<string | null>(() => initialTrack(subtitles));
  const [flash, setFlash] = useState<{ id: number; text: string; side?: "left" | "right" } | null>(
    null,
  );
  const idleTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const lastTap = useRef<{ at: number; zone: "left" | "center" | "right" } | null>(null);

  useImperativeHandle(ref, () => ({
    currentTime: () => element.current?.currentTime ?? 0,
    duration: () =>
      element.current && Number.isFinite(element.current.duration)
        ? element.current.duration
        : source.durationSeconds,
    paused: () => element.current?.paused ?? true,
    play: controls.play,
    pause: controls.pause,
    seek: controls.seek,
    setRate: controls.setRate,
    element: () => element.current,
  }));

  const wake = () => {
    setActive(true);
    clearTimeout(idleTimer.current);
    idleTimer.current = setTimeout(() => setActive(false), IDLE_MS);
  };
  useEffect(() => () => clearTimeout(idleTimer.current), []);

  const announce = (text: string, side?: "left" | "right") =>
    setFlash({ id: Date.now(), text, side });
  useEffect(() => {
    if (!flash) return;
    const timer = setTimeout(() => setFlash(null), 700);
    return () => clearTimeout(timer);
  }, [flash]);

  useEffect(() => {
    const onChange = () => setFullscreen(document.fullscreenElement === container.current);
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, []);

  useEffect(() => {
    const media = element.current;
    if (!media) return;
    const enter = () => setPip(true);
    const leave = () => setPip(false);
    media.addEventListener("enterpictureinpicture", enter);
    media.addEventListener("leavepictureinpicture", leave);
    return () => {
      media.removeEventListener("enterpictureinpicture", enter);
      media.removeEventListener("leavepictureinpicture", leave);
    };
  }, []);

  const toggleFullscreen = () => {
    const box = container.current;
    const media = element.current as
      | (HTMLVideoElement & { webkitEnterFullscreen?: () => void })
      | null;
    if (document.fullscreenElement) void document.exitFullscreen();
    else if (box?.requestFullscreen) void box.requestFullscreen().catch(() => {});
    // iPhones only let the video element itself go fullscreen.
    else media?.webkitEnterFullscreen?.();
  };
  const pipSupported = typeof document !== "undefined" && document.pictureInPictureEnabled;
  const togglePip = () => {
    if (document.pictureInPictureElement) void document.exitPictureInPicture().catch(() => {});
    else void element.current?.requestPictureInPicture().catch(() => {});
  };

  const selectTrack = (id: string | null) => {
    setTrackId(id);
    try {
      window.localStorage.setItem(
        CAPTIONS_KEY,
        id ? (subtitles.find((track) => track.id === id)?.language ?? "") : "off",
      );
    } catch {
      // Private mode: the choice lasts for this page.
    }
  };
  const toggleCaptions = () => {
    if (subtitles.length === 0) return;
    if (trackId) selectTrack(null);
    else
      selectTrack(
        (
          subtitles.find((track) => track.language === storedCaptions()) ??
          subtitles.find((track) => track.isDefault) ??
          subtitles[0]
        )?.id ?? null,
      );
  };

  const run = (command: PlayerCommand) => {
    const transport =
      command.type === "toggle" ||
      command.type === "seekBy" ||
      command.type === "seekToFraction" ||
      command.type === "rateBy";
    if (transport && locked) {
      announce(lockedReason ?? "");
      return;
    }
    switch (command.type) {
      case "toggle":
        controls.toggle();
        break;
      case "seekBy":
        controls.seekBy(command.seconds);
        announce(
          t(command.seconds < 0 ? "player.back" : "player.forward", {
            seconds: Math.abs(command.seconds),
          }),
          command.seconds < 0 ? "left" : "right",
        );
        break;
      case "seekToFraction":
        controls.seek(state.duration * command.fraction);
        break;
      case "volumeBy": {
        const next =
          Math.round(
            Math.min(1, Math.max(0, (state.muted ? 0 : state.volume) + command.delta)) * 100,
          ) / 100;
        controls.setVolume(next);
        announce(t("player.volumeAt", { percent: Math.round(next * 100) }));
        break;
      }
      case "mute":
        controls.setMuted(!state.muted);
        announce(state.muted ? t("player.unmuted") : t("player.muted"));
        break;
      case "fullscreen":
        toggleFullscreen();
        break;
      case "theatre":
        onTheatreChange?.(!theatre);
        break;
      case "captions":
        toggleCaptions();
        break;
      case "pip":
        if (pipSupported) togglePip();
        break;
      case "rateBy": {
        const next = stepRate(state.rate, command.delta);
        controls.setRate(next);
        announce(`${next}×`);
        break;
      }
    }
    wake();
  };

  // The page's keys belong to the player while it is on screen. `run` reads fresh state through this ref.
  const runRef = useRef(run);
  useEffect(() => {
    runRef.current = run;
  });
  useEffect(() => {
    if (!shortcuts) return;
    const onKey = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        !playerOwnsKey(event.target instanceof Element ? event.target : null, event)
      )
        return;
      const command = commandForKey(event);
      if (!command) return;
      event.preventDefault();
      runRef.current(command);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [shortcuts]);

  const visible = !state.started || state.paused || state.ended || active || scrubbing || menuOpen;
  const zoneOf = (event: { clientX: number }): "left" | "center" | "right" => {
    const box = container.current?.getBoundingClientRect();
    if (!box) return "center";
    const x = (event.clientX - box.left) / box.width;
    return x < 0.35 ? "left" : x > 0.65 ? "right" : "center";
  };
  const onSurfaceClick = (event: MouseEvent) => {
    // Fingers are handled on pointer-up (taps and double-taps mean other things).
    if ((event.nativeEvent as PointerEvent).pointerType === "touch") return;
    if (menuOpen) setMenuOpen(false);
    else run({ type: "toggle" });
  };
  const onSurfaceTouch = (event: ReactPointerEvent) => {
    if (event.pointerType !== "touch") return;
    if (menuOpen) {
      setMenuOpen(false);
      return;
    }
    const now = Date.now();
    const zone = zoneOf(event);
    const previous = lastTap.current;
    lastTap.current = { at: now, zone };
    if (previous && now - previous.at < DOUBLE_TAP_MS && previous.zone === zone) {
      // Double-tap: the sides jump, the middle plays or pauses. Further taps keep jumping.
      run(
        zone === "center"
          ? { type: "toggle" }
          : { type: "seekBy", seconds: zone === "left" ? -10 : 10 },
      );
      return;
    }
    if (visible && !state.paused) {
      clearTimeout(idleTimer.current);
      setActive(false);
    } else wake();
  };

  const chapter = chapterAt(chapters, state.time);
  const track = subtitles.find((candidate) => candidate.id === trackId);
  const VolumeIcon =
    state.muted || state.volume === 0 ? VolumeX : state.volume < 0.5 ? Volume1 : Volume2;

  return (
    // biome-ignore lint/a11y/useSemanticElements: a region grouping the video with its custom controls.
    <div
      ref={container}
      role="group"
      aria-label={title ? t("player.labelFor", { title }) : t("player.label")}
      data-testid="video-player"
      data-state={
        state.failed
          ? "failed"
          : state.ended
            ? "ended"
            : state.paused
              ? "paused"
              : state.waiting
                ? "buffering"
                : "playing"
      }
      className={cn(
        "@container relative isolate overflow-hidden bg-black text-white select-none",
        fullscreen ? "h-full w-full" : "aspect-video w-full",
        !visible && "cursor-none",
        className,
      )}
      onPointerMove={(event) => event.pointerType === "mouse" && wake()}
      onFocus={wake}
    >
      <video
        ref={element}
        playsInline
        preload="metadata"
        poster={source.poster?.url || undefined}
        className="absolute inset-0 size-full object-contain"
        onPlay={onPlay}
        onPause={onPause}
        onSeeked={(event) => onSeeked?.(event.currentTarget.currentTime)}
        onEnded={onEnded}
        onRateChange={(event) => onRateChange?.(event.currentTarget.playbackRate)}
        onWaiting={() => onWaiting?.(true)}
        onPlaying={() => onWaiting?.(false)}
      />

      {/* The surface under the controls: click or tap the picture itself. */}
      <div
        className="absolute inset-0"
        onClick={onSurfaceClick}
        onDoubleClick={(event) =>
          (event.nativeEvent as PointerEvent).pointerType !== "touch" && toggleFullscreen()
        }
        onPointerUp={onSurfaceTouch}
      />

      {track && <CaptionLayer media={element} track={track} raised={visible} />}

      {state.waiting && !state.paused && !state.failed && (
        <div className="pointer-events-none absolute inset-0 grid place-items-center">
          <Spinner className="size-12" />
        </div>
      )}

      {flash && (
        <div
          key={flash.id}
          aria-hidden
          className={cn(
            "pointer-events-none absolute top-1/2 flex -translate-y-1/2 animate-fade-in items-center gap-1.5 rounded-full bg-black/65 px-4 py-2 text-sm font-black",
            flash.side === "left"
              ? "left-[12%]"
              : flash.side === "right"
                ? "right-[12%]"
                : "left-1/2 -translate-x-1/2",
          )}
        >
          {flash.side === "left" && <Rewind className="size-4 fill-current" />}
          {flash.text}
          {flash.side === "right" && <FastForward className="size-4 fill-current" />}
        </div>
      )}

      {!state.started && !state.failed && !locked && (
        <button
          type="button"
          aria-label={t("player.play")}
          className="pressable sticker absolute top-1/2 left-1/2 grid size-16 -translate-x-1/2 -translate-y-1/2 place-items-center rounded-full bg-brand text-on-brand"
          onClick={() => controls.toggle()}
        >
          <Play className="size-7 translate-x-0.5 fill-current" />
        </button>
      )}

      {lockedReason && (
        <p
          className={cn(
            "pointer-events-none absolute top-3 left-3 flex max-w-[80%] items-center gap-1.5 rounded-full bg-black/65 px-3 py-1.5 text-xs font-bold transition-opacity",
            visible ? "opacity-100" : "opacity-0",
          )}
        >
          <Lock className="size-3.5 shrink-0" />
          <span className="truncate">{lockedReason}</span>
        </p>
      )}
      {fullscreen && title && !lockedReason && (
        <p
          className={cn(
            "pointer-events-none absolute inset-x-0 top-0 truncate bg-gradient-to-b from-black/70 to-transparent px-5 pt-4 pb-10 text-lg font-black transition-opacity",
            visible ? "opacity-100" : "opacity-0",
          )}
        >
          {title}
        </p>
      )}

      {state.failed && (
        <div
          role="alert"
          className="absolute inset-0 grid place-items-center bg-black/80 p-6 text-center"
        >
          <div>
            <p className="font-black">{t("player.failed")}</p>
            <button
              type="button"
              className="pressable mt-4 inline-flex h-11 items-center gap-2 rounded-full bg-white px-5 font-bold text-[#17122b]"
              onClick={controls.retry}
            >
              <RotateCcw className="size-4" />
              {t("player.retry")}
            </button>
          </div>
        </div>
      )}

      {state.ended && !state.failed && (
        <div className="absolute inset-0 grid place-items-center bg-black/70 p-4">
          {endScreen ??
            (!locked && (
              <button
                type="button"
                className="pressable inline-flex h-12 items-center gap-2 rounded-full bg-white px-6 font-black text-[#17122b]"
                onClick={() => controls.toggle()}
              >
                <RotateCcw className="size-5" />
                {t("player.replay")}
              </button>
            ))}
        </div>
      )}

      {menuOpen && (
        <SettingsMenu
          state={state}
          subtitles={subtitles}
          trackId={trackId}
          rateLocked={locked}
          onRate={controls.setRate}
          onLevel={controls.setLevel}
          onTrack={selectTrack}
          onClose={() => setMenuOpen(false)}
        />
      )}

      <div
        data-testid="player-controls"
        className={cn(
          "absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/80 via-black/40 to-transparent px-2 pt-10 pb-1 transition-opacity duration-200 sm:px-3",
          visible ? "opacity-100" : "pointer-events-none opacity-0",
        )}
      >
        <Scrubber
          media={element}
          time={state.time}
          duration={state.duration}
          bufferedEnd={state.bufferedEnd}
          chapters={chapters}
          storyboardUrl={source.storyboardUrl}
          label={t("player.seek")}
          disabled={locked}
          onSeek={controls.seek}
          onScrubbing={setScrubbing}
        />
        <div className="flex h-11 items-center gap-0.5">
          <ControlButton
            label={
              state.ended ? t("player.replay") : state.paused ? t("player.play") : t("player.pause")
            }
            disabled={locked}
            onClick={() => run({ type: "toggle" })}
          >
            {state.ended ? (
              <RotateCcw className="size-5" />
            ) : state.paused ? (
              <Play className="size-5 fill-current" />
            ) : (
              <Pause className="size-5 fill-current" />
            )}
          </ControlButton>
          <ControlButton
            label={state.muted ? t("player.unmute") : t("player.mute")}
            onClick={() => controls.setMuted(!state.muted)}
          >
            <VolumeIcon className="size-5" />
          </ControlButton>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={state.muted ? 0 : state.volume}
            aria-label={t("player.volume")}
            className="mr-1 h-1 w-20 cursor-pointer accent-white @max-xl:hidden"
            onChange={(event) => controls.setVolume(Number(event.target.value))}
          />
          <p className="min-w-0 truncate px-1.5 text-xs font-bold tabular-nums sm:text-sm">
            <span data-testid="player-time">{formatClock(state.time)}</span>
            <span className="text-white/70"> / {formatClock(state.duration)}</span>
            {chapter && <span className="text-white/85"> ・ {chapter.title}</span>}
          </p>
          <span className="flex-1" />
          {subtitles.length > 0 && (
            <ControlButton
              label={trackId ? t("player.captionsOff") : t("player.captionsOn")}
              pressed={Boolean(trackId)}
              onClick={toggleCaptions}
            >
              {trackId ? <Captions className="size-5" /> : <CaptionsOff className="size-5" />}
            </ControlButton>
          )}
          <ControlButton
            label={t("player.settings")}
            expanded={menuOpen}
            onClick={() => setMenuOpen((open) => !open)}
          >
            <Settings className={cn("size-5 transition-transform", menuOpen && "rotate-45")} />
          </ControlButton>
          {pipSupported && (
            <ControlButton
              label={t("player.pip")}
              pressed={pip}
              className="@max-md:hidden"
              onClick={togglePip}
            >
              <PictureInPicture2 className="size-5" />
            </ControlButton>
          )}
          {onTheatreChange && !fullscreen && (
            <ControlButton
              label={theatre ? t("player.theatreOff") : t("player.theatreOn")}
              pressed={theatre}
              className="max-lg:hidden"
              onClick={() => onTheatreChange(!theatre)}
            >
              <RectangleHorizontal className="size-5" />
            </ControlButton>
          )}
          <ControlButton
            label={fullscreen ? t("player.fullscreenOff") : t("player.fullscreenOn")}
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
  pressed,
  expanded,
  disabled,
  className,
  onClick,
  children,
}: {
  label: string;
  pressed?: boolean;
  expanded?: boolean;
  disabled?: boolean;
  className?: string;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={pressed}
      aria-expanded={expanded}
      disabled={disabled}
      className={cn(
        "grid size-10 shrink-0 place-items-center rounded-full transition-colors hover:bg-white/15 disabled:opacity-40",
        className,
      )}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

/** Loads a subtitle track and shows the cue of the moment, in step with the picture. */
function CaptionLayer({
  media,
  track,
  raised,
}: {
  media: RefObject<HTMLVideoElement | null>;
  track: video.SubtitleTrack;
  raised: boolean;
}) {
  const cues = useQuery({
    queryKey: ["video", "captions", track.url],
    queryFn: async (): Promise<Cue[]> => {
      const response = await fetch(scriptUrl(track.url));
      if (!response.ok) throw new Error(`subtitles failed (${response.status})`);
      return parseVtt(await response.text());
    },
    staleTime: Number.POSITIVE_INFINITY,
  });
  const [text, setText] = useState("");
  const loaded = cues.data;

  useEffect(() => {
    if (!loaded) return;
    let frame = 0;
    const tick = () => {
      const next = activeCues(loaded, media.current?.currentTime ?? 0)
        .map((cue) => cue.text)
        .join("\n");
      setText((current) => (current === next ? current : next));
      frame = requestAnimationFrame(tick);
    };
    tick();
    return () => cancelAnimationFrame(frame);
  }, [loaded, media]);

  if (!text) return null;
  return (
    <div
      className={cn(
        "pointer-events-none absolute inset-x-0 flex justify-center px-4 transition-[bottom] duration-200",
        raised ? "bottom-20" : "bottom-6",
      )}
    >
      <p
        lang={track.language}
        data-testid="player-captions"
        className="max-w-[90%] rounded-lg bg-black/75 px-3 py-1 text-center text-[clamp(0.875rem,2.4cqw,1.5rem)] leading-snug font-bold whitespace-pre-line"
      >
        {text}
      </p>
    </div>
  );
}
