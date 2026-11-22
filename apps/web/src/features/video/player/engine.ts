import type Hls from "hls.js";
import { type RefObject, useEffect, useRef, useState } from "react";
import { scriptUrl } from "./media-url";

/** One rung of the quality ladder, as the menu lists it. */
export interface QualityLevel {
  /** Index in the stream's level list (what `setLevel` takes). */
  index: number;
  height: number;
}

/** Everything the controls show about the media element. */
export interface MediaState {
  /** The length and size are known. */
  ready: boolean;
  paused: boolean;
  /** Playback has begun at least once. */
  started: boolean;
  ended: boolean;
  /** Stalled waiting for data. */
  waiting: boolean;
  time: number;
  duration: number;
  /** How far the data in memory reaches from the current position. */
  bufferedEnd: number;
  volume: number;
  muted: boolean;
  rate: number;
  /** The stream cannot be played (after recovery was tried). */
  failed: boolean;
  levels: QualityLevel[];
  /** The chosen level; -1 lets the player adapt to the connection. */
  level: number;
  /** The level actually playing (while adapting). */
  playingLevel: number;
}

export interface MediaSource {
  hlsUrl: string;
  mp4Url: string;
  durationSeconds: number;
}

export interface MediaControls {
  play(): Promise<void>;
  pause(): void;
  toggle(): void;
  seek(seconds: number): void;
  seekBy(seconds: number): void;
  setVolume(volume: number): void;
  setMuted(muted: boolean): void;
  setRate(rate: number): void;
  setLevel(level: number): void;
  /** Loads the stream again after a failure. */
  retry(): void;
}

const VOLUME_KEY = "tokuly.player.volume";
const MUTED_KEY = "tokuly.player.muted";

function stored(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function store(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Private mode: the setting lasts for this page.
  }
}

function bufferedEnd(video: HTMLVideoElement): number {
  const { buffered, currentTime } = video;
  for (let index = 0; index < buffered.length; index++) {
    if (buffered.start(index) <= currentTime + 0.25 && buffered.end(index) >= currentTime)
      return buffered.end(index);
  }
  return currentTime;
}

/**
 * Plays a processed upload in a `<video>`: adaptive HLS through hls.js
 * (loaded on demand), natively where the browser speaks HLS itself, and the
 * progressive MP4 as the last resort. Mirrors the element's state for the
 * controls and remembers volume and mute between visits.
 */
export function useMediaEngine(
  video: RefObject<HTMLVideoElement | null>,
  source: MediaSource,
  options: { startAt: number; autoPlay: boolean },
): [MediaState, MediaControls] {
  const hls = useRef<Hls | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<MediaState>(() => ({
    ready: false,
    paused: true,
    started: false,
    ended: false,
    waiting: false,
    time: options.startAt,
    duration: source.durationSeconds,
    bufferedEnd: 0,
    volume: Math.min(1, Math.max(0, Number(stored(VOLUME_KEY) ?? "1") || 0)),
    muted: stored(MUTED_KEY) === "1",
    rate: 1,
    failed: false,
    levels: [],
    level: -1,
    playingLevel: -1,
  }));
  // Read when the stream is attached; later changes of these props do not restart it.
  const initial = useRef(options);
  const startVolume = useRef({ volume: state.volume, muted: state.muted });

  const { hlsUrl, mp4Url, durationSeconds } = source;
  // `attempt` restarts loading after a failure.
  useEffect(() => {
    const element = video.current;
    if (!element) return;
    let disposed = false;
    const patch = (next: Partial<MediaState>) =>
      !disposed && setState((current) => ({ ...current, ...next }));
    const { startAt, autoPlay } = initial.current;
    element.volume = startVolume.current.volume;
    element.muted = startVolume.current.muted;

    const begin = () => {
      if (autoPlay) void element.play().catch(() => {});
    };
    const listeners: [keyof HTMLMediaElementEventMap, () => void][] = [
      [
        "loadedmetadata",
        () =>
          patch({
            ready: true,
            duration:
              Number.isFinite(element.duration) && element.duration > 0
                ? element.duration
                : durationSeconds,
          }),
      ],
      [
        "durationchange",
        () =>
          Number.isFinite(element.duration) &&
          element.duration > 0 &&
          patch({ duration: element.duration }),
      ],
      ["play", () => patch({ paused: false, started: true, ended: false })],
      ["pause", () => patch({ paused: true })],
      ["playing", () => patch({ waiting: false, paused: false })],
      ["waiting", () => patch({ waiting: true })],
      ["canplay", () => patch({ waiting: false })],
      ["seeking", () => patch({ time: element.currentTime, ended: false })],
      ["seeked", () => patch({ time: element.currentTime, waiting: false })],
      ["timeupdate", () => patch({ time: element.currentTime, bufferedEnd: bufferedEnd(element) })],
      ["progress", () => patch({ bufferedEnd: bufferedEnd(element) })],
      ["ended", () => patch({ ended: true, paused: true })],
      ["ratechange", () => patch({ rate: element.playbackRate })],
      ["volumechange", () => patch({ volume: element.volume, muted: element.muted })],
    ];
    for (const [name, listener] of listeners) element.addEventListener(name, listener);

    const playMp4 = () => {
      if (!mp4Url) return patch({ failed: true });
      element.src = mp4Url;
      if (startAt > 0) element.currentTime = startAt;
      begin();
    };
    const onNativeError = () => patch({ failed: true });
    let resumeLoading = () => {};

    if (!hlsUrl) {
      element.addEventListener("error", onNativeError);
      playMp4();
    } else if (
      element.canPlayType("application/vnd.apple.mpegurl") &&
      !("MediaSource" in window || "ManagedMediaSource" in window)
    ) {
      // Browsers without Media Source Extensions (iPhones) play HLS themselves.
      element.addEventListener("error", onNativeError);
      element.src = hlsUrl;
      if (startAt > 0)
        element.addEventListener("loadedmetadata", () => (element.currentTime = startAt), {
          once: true,
        });
      begin();
    } else {
      void import("hls.js").then(({ default: HlsPlayer }) => {
        if (disposed) return;
        if (!HlsPlayer.isSupported()) {
          element.addEventListener("error", onNativeError);
          playMp4();
          return;
        }
        const player = new HlsPlayer({
          startPosition: startAt > 0 ? startAt : -1,
          capLevelToPlayerSize: true,
          maxBufferLength: 30,
        });
        hls.current = player;
        let mediaRecoveries = 0;
        // Back online after a dropped connection: pick the stream up again (unless the player is gone by then).
        resumeLoading = () => hls.current === player && player.startLoad();
        player.on(HlsPlayer.Events.MANIFEST_PARSED, () => {
          patch({
            levels: player.levels
              .map((level, index) => ({ index, height: level.height }))
              .filter((level) => level.height > 0),
          });
          begin();
        });
        player.on(HlsPlayer.Events.LEVEL_SWITCHED, (_event, data) =>
          patch({ playingLevel: data.level }),
        );
        player.on(HlsPlayer.Events.ERROR, (_event, data) => {
          if (!data.fatal) return;
          // One more try for what usually passes: a dropped connection, a decoder hiccup.
          if (data.type === HlsPlayer.ErrorTypes.MEDIA_ERROR && mediaRecoveries < 2) {
            mediaRecoveries += 1;
            player.recoverMediaError();
          } else if (
            data.type === HlsPlayer.ErrorTypes.NETWORK_ERROR &&
            navigator.onLine === false
          ) {
            window.removeEventListener("online", resumeLoading);
            window.addEventListener("online", resumeLoading, { once: true });
          } else {
            player.destroy();
            hls.current = null;
            patch({ failed: true });
          }
        });
        player.loadSource(scriptUrl(hlsUrl));
        player.attachMedia(element);
      });
    }

    return () => {
      disposed = true;
      for (const [name, listener] of listeners) element.removeEventListener(name, listener);
      element.removeEventListener("error", onNativeError);
      window.removeEventListener("online", resumeLoading);
      hls.current?.destroy();
      hls.current = null;
      element.removeAttribute("src");
      element.load();
    };
  }, [video, hlsUrl, mp4Url, durationSeconds, attempt]);

  const element = () => video.current;
  const controls: MediaControls = {
    play: async () => {
      await element()?.play();
    },
    pause: () => element()?.pause(),
    toggle: () => {
      const media = element();
      if (!media) return;
      if (media.paused || media.ended) void media.play().catch(() => {});
      else media.pause();
    },
    seek: (seconds) => {
      const media = element();
      if (!media) return;
      const length =
        Number.isFinite(media.duration) && media.duration > 0 ? media.duration : state.duration;
      media.currentTime = Math.min(Math.max(0, seconds), Math.max(0, length - 0.05));
    },
    seekBy: (seconds) => {
      const media = element();
      if (media) controls.seek(media.currentTime + seconds);
    },
    setVolume: (volume) => {
      const media = element();
      if (!media) return;
      const next = Math.min(1, Math.max(0, volume));
      media.volume = next;
      media.muted = next === 0;
      store(VOLUME_KEY, String(next));
      store(MUTED_KEY, next === 0 ? "1" : "0");
    },
    setMuted: (muted) => {
      const media = element();
      if (!media) return;
      media.muted = muted;
      // Unmuting at zero volume would still be silent.
      if (!muted && media.volume === 0) media.volume = 0.5;
      store(MUTED_KEY, muted ? "1" : "0");
    },
    setRate: (rate) => {
      const media = element();
      if (media) media.playbackRate = rate;
    },
    setLevel: (level) => {
      if (!hls.current) return;
      hls.current.currentLevel = level;
      setState((current) => ({ ...current, level }));
    },
    retry: () => {
      // Pick up where it broke off.
      initial.current = { startAt: element()?.currentTime || state.time, autoPlay: true };
      setState((current) => ({ ...current, failed: false, waiting: false }));
      setAttempt((count) => count + 1);
    },
  };
  return [state, controls];
}
