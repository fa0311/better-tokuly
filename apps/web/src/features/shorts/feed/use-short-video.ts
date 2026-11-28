import type { media } from "@tokuly/api";
import type Hls from "hls.js";
import { type RefObject, useEffect, useRef, useState } from "react";
import { create } from "zustand";
import { loadHls } from "#/lib/hls";
import { StayMeter } from "../stay";

const MUTED_KEY = "tokuly.shorts.muted";

function storedMuted(): boolean {
  try {
    return window.localStorage.getItem(MUTED_KEY) === "1";
  } catch {
    return false;
  }
}

/**
 * The sound of the swipe feed, shared by every short: `muted` is the
 * viewer's choice and is remembered; `blocked` is the browser refusing sound
 * before the first tap on the page, which the next tap lifts.
 */
export const useFeedSound = create<{ muted: boolean; blocked: boolean }>(() => ({
  muted: typeof window === "undefined" ? false : storedMuted(),
  blocked: false,
}));

export function setFeedMuted(muted: boolean): void {
  useFeedSound.setState({ muted, blocked: false });
  try {
    window.localStorage.setItem(MUTED_KEY, muted ? "1" : "0");
  } catch {
    // Private mode: the choice lasts for this page.
  }
}

export interface ShortVideoState {
  /** Paused by the viewer. */
  paused: boolean;
  /** Stalled waiting for data, or not started yet. */
  waiting: boolean;
  /** The stream cannot be played. */
  failed: boolean;
}

export interface ShortVideoControls {
  toggle(): void;
  /** Double speed while held. */
  setFast(fast: boolean): void;
  /** How the stay went, up to now; a new stay starts counting. */
  takeStay(): { watchedMs: number; completions: number };
}

/**
 * Drives the `<video>` of one slide. The element gets its stream as soon as
 * it is mounted (so the next short loads ahead) and gives it back when it
 * goes: the source is detached and the buffers are freed, which matters on
 * phones. It plays only while `active`, looping, and measures the stay.
 */
export function useShortVideo(
  video: RefObject<HTMLVideoElement | null>,
  playback: Pick<media.VideoInfo, "mp4Url" | "hlsUrl"> | undefined,
  active: boolean,
  progress: RefObject<HTMLElement | null>,
): [ShortVideoState, ShortVideoControls] {
  const [state, setState] = useState<ShortVideoState>({
    paused: false,
    waiting: true,
    failed: false,
  });
  const meter = useRef(new StayMeter());
  const wantsPause = useRef(false);
  const muted = useFeedSound((sound) => sound.muted || sound.blocked);
  const mp4Url = playback?.mp4Url ?? "";
  const hlsUrl = playback?.hlsUrl ?? "";

  // The stream: attached on mount, released on unmount.
  useEffect(() => {
    const element = video.current;
    if (!element || (!mp4Url && !hlsUrl)) return;
    let disposed = false;
    let player: Hls | null = null;
    const fail = () => !disposed && setState((current) => ({ ...current, failed: true }));
    element.addEventListener("error", fail);
    if (mp4Url) {
      // Shorts are a few megabytes at most: the progressive file starts fastest and holds no extra buffers.
      element.src = mp4Url;
    } else if (element.canPlayType("application/vnd.apple.mpegurl")) {
      element.src = hlsUrl;
    } else {
      loadHls().then(({ default: HlsPlayer }) => {
        if (disposed) return;
        if (!HlsPlayer.isSupported()) return fail();
        player = new HlsPlayer({ capLevelToPlayerSize: true, maxBufferLength: 10 });
        player.on(HlsPlayer.Events.ERROR, (_event, data) => {
          if (!data.fatal) return;
          player?.destroy();
          player = null;
          fail();
        });
        player.loadSource(hlsUrl);
        player.attachMedia(element);
      }, fail);
    }
    return () => {
      disposed = true;
      element.removeEventListener("error", fail);
      player?.destroy();
      element.pause();
      element.removeAttribute("src");
      element.load();
    };
  }, [video, mp4Url, hlsUrl]);

  // Sound follows the feed's setting.
  useEffect(() => {
    if (video.current) video.current.muted = muted;
  }, [video, muted]);

  // Playback: only the short on screen plays; it starts over each time it comes back.
  useEffect(() => {
    const element = video.current;
    if (!element) return;
    // Read on every event: `takeStay` swaps the meter for a new one.
    const stay = {
      play: (at: number) => meter.current.play(at),
      pause: (at: number) => meter.current.pause(at),
      complete: (at: number) => meter.current.complete(at),
    };
    const now = () => performance.now();
    const patch = (next: Partial<ShortVideoState>) =>
      setState((current) => ({ ...current, ...next }));

    const start = () => {
      if (wantsPause.current) return;
      void element.play().catch((error: unknown) => {
        // Sound before the first tap is refused: play silently and offer the sound.
        if (!(error instanceof DOMException) || error.name !== "NotAllowedError" || element.muted)
          return;
        useFeedSound.setState({ blocked: true });
        element.muted = true;
        void element.play().catch(() => {});
      });
    };
    const listeners: [keyof HTMLMediaElementEventMap, () => void][] = [
      [
        "playing",
        () => {
          stay.play(now());
          patch({ waiting: false, paused: false });
        },
      ],
      [
        "waiting",
        () => {
          stay.pause(now());
          patch({ waiting: true });
        },
      ],
      ["pause", () => stay.pause(now())],
      [
        "ended",
        () => {
          stay.complete(now());
          element.currentTime = 0;
          start();
        },
      ],
      [
        "timeupdate",
        () => {
          // Written straight to the bar: no render per frame.
          const length = element.duration;
          if (progress.current && Number.isFinite(length) && length > 0)
            progress.current.style.transform = `scaleX(${Math.min(1, element.currentTime / length)})`;
        },
      ],
    ];
    if (!active) {
      element.pause();
      return;
    }
    for (const [name, listener] of listeners) element.addEventListener(name, listener);
    wantsPause.current = false;
    patch({ paused: false });
    element.playbackRate = 1;
    if (element.currentTime > 0) element.currentTime = 0;
    start();
    // A tab in the background does not watch.
    const onVisibility = () => (document.hidden ? element.pause() : start());
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      for (const [name, listener] of listeners) element.removeEventListener(name, listener);
      stay.pause(now());
      element.pause();
    };
  }, [video, active, progress]);

  // Reads refs only, so one function serves every render (the report on leaving uses it from an effect).
  const [takeStay] = useState(() => () => {
    const stay = meter.current.read(performance.now());
    meter.current = new StayMeter();
    if (video.current && !video.current.paused) meter.current.play(performance.now());
    return stay;
  });

  const controls: ShortVideoControls = {
    toggle: () => {
      const element = video.current;
      if (!element) return;
      wantsPause.current = !wantsPause.current;
      setState((current) => ({ ...current, paused: wantsPause.current }));
      if (wantsPause.current) element.pause();
      else void element.play().catch(() => {});
    },
    setFast: (fast) => {
      if (video.current) video.current.playbackRate = fast ? 2 : 1;
    },
    takeStay,
  };
  return [state, controls];
}
